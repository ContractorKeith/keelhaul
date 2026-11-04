use super::*;
use rusty_s3::S3Action;
use std::time::Duration;

#[test]
fn s3_config_debug_redacts_credentials() {
    let config = S3Config::new(
        S3Location::parse("s3:https://example.com/bucket").unwrap(),
        Credentials::new_with_token("ACCESS12345678", "very-secret", "session-secret"),
    );
    let debug = format!("{config:?}");
    assert!(debug.contains("****5678"));
    assert!(!debug.contains("ACCESS12345678"));
    for secret in ["ACCESS1234", "very-secret", "session-secret", "Signature"] {
        assert!(!debug.contains(secret));
    }
    let backend = S3Backend::new(config).unwrap();
    assert!(!format!("{backend:?}").contains("ACCESS"));
}

#[test]
fn s3_request_path_headers_and_pagination_token() {
    let location = S3Location::parse("s3:http://localhost:7070/bucket/prefix").unwrap();
    assert_eq!(location.prefix, "prefix/");
    assert_eq!(location.endpoint, "http://localhost:7070");
    assert_eq!(location.bucket, "bucket");
    let bucket = location.bucket("us-east-1").unwrap();
    let credentials = Credentials::new("access", "secret");
    let mut put = bucket.put_object(Some(&credentials), "prefix/config");
    put.headers_mut().insert("if-none-match", "*");
    let url = put.sign(Duration::from_secs(60));
    assert_eq!(url.path(), "/bucket/prefix/config");
    assert!(
        url.query_pairs()
            .any(|(k, v)| k == "X-Amz-SignedHeaders" && v.contains("if-none-match"))
    );
    let mut list = bucket.list_objects_v2(Some(&credentials));
    list.with_prefix("prefix/index/");
    list.with_continuation_token("a+/=&token");
    let url = list.sign(Duration::from_secs(60));
    assert!(
        url.query_pairs()
            .any(|(k, v)| k == "continuation-token" && v == "a+/=&token")
    );
}

#[test]
fn s3_captured_versitygw_pages_parse() {
    use rusty_s3::actions::ListObjectsV2;
    let first = ListObjectsV2::parse_response(include_str!("fixtures/list-page-0.xml")).unwrap();
    let second = ListObjectsV2::parse_response(include_str!("fixtures/list-page-1.xml")).unwrap();
    assert_eq!(first.contents.len(), 1000);
    assert_eq!(second.contents.len(), 3);
    assert!(first.next_continuation_token.is_some());
    assert!(second.next_continuation_token.is_none());
    assert_eq!(first.contents[0].key, format!("index/{:064x}", 0));
    assert_eq!(second.contents[2].key, format!("index/{:064x}", 1002));
    assert_eq!(second.contents[2].size, Some(1));
    assert!(second.contents[2].last_modified.is_some());
    for xml in [
        "<invalid>",
        r#"<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/"><IsTruncated>true</IsTruncated></ListBucketResult>"#,
    ] {
        assert!(ListObjectsV2::parse_response(xml).is_err());
    }
}

fn mock(responses: Vec<(u16, &'static str)>) -> (S3Backend, std::thread::JoinHandle<Vec<String>>) {
    mock_headers(
        responses
            .into_iter()
            .map(|(status, body)| (status, body, ""))
            .collect(),
    )
}

pub(super) fn mock_headers(
    responses: Vec<(u16, &'static str, &'static str)>,
) -> (S3Backend, std::thread::JoinHandle<Vec<String>>) {
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("s3:http://{}/bucket/repo", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body, extra) in responses {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut header = Vec::new();
            loop {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                header.push(byte[0]);
                if header.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let header = String::from_utf8(header).unwrap();
            let size = header
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                        .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut bytes = vec![0; size];
            stream.read_exact(&mut bytes).unwrap();
            let head = header.starts_with("HEAD ");
            requests.push(header);
            write!(
                stream,
                "HTTP/1.1 {status} Mock\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            if !head {
                stream.write_all(body.as_bytes()).unwrap();
            }
        }
        requests
    });
    let backend = S3Backend::new({
        let mut config = S3Config::new(
            S3Location::parse(&endpoint).unwrap(),
            Credentials::new("key", "secret"),
        );
        config.timeout = Duration::from_secs(3);
        config
    })
    .unwrap();
    (backend, worker)
}

#[test]
fn s3_ignored_conditional_header_falls_back_once() {
    let (backend, worker) = mock(vec![
        (200, ""),
        (200, ""),
        (204, ""), // probe detects ignored header, then cleans up
        (404, ""),
        (200, ""), // HEAD then PUT new object
        (200, "abc"),
        (200, "abc"), // identical retry: HEAD and GET
        (200, "abc"),
        (200, "abc"), // conflicting retry: HEAD and GET
    ]);
    let name = ObjectName::new("config").unwrap();
    backend.write(&name, b"abc").unwrap();
    backend.write(&name, b"abc").unwrap();
    assert!(matches!(
        backend.write(&name, b"xyz"),
        Err(BackendError::AlreadyExists(_))
    ));
    let requests = worker.join().unwrap();
    assert_eq!(requests.len(), 9);
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.to_lowercase().contains("if-none-match:"))
            .count(),
        1
    );
    assert!(requests[3].starts_with("HEAD /bucket/repo/config?"));
    assert!(requests[4].starts_with("PUT /bucket/repo/config?"));
}

#[test]
fn s3_unsupported_conditional_header_falls_back() {
    for status in [400, 401, 403, 404, 405, 409] {
        let (backend, worker) = mock(vec![
            (200, ""),
            (status, ""),
            (204, ""),
            (404, ""),
            (200, ""),
        ]);
        backend
            .write(&ObjectName::new("config").unwrap(), b"abc")
            .unwrap();
        assert_eq!(worker.join().unwrap().len(), 5);
    }
}

#[test]
fn s3_retry_policy_is_bounded_and_never_retries_other_4xx() {
    let name = ObjectName::new("config").unwrap();
    for status in [400, 401, 403, 404, 409, 412] {
        let (backend, worker) = mock(vec![(status, "sensitive-response")]);
        let error = backend.read(&name).unwrap_err();
        assert!(!error.to_string().contains("sensitive-response"));
        assert_eq!(worker.join().unwrap().len(), 1);
    }
    let (backend, worker) = mock(vec![(429, ""), (500, ""), (503, ""), (200, "value")]);
    assert_eq!(backend.read(&name).unwrap(), b"value");
    assert_eq!(worker.join().unwrap().len(), 4);
    let (backend, worker) = mock(vec![(503, ""); 5]);
    assert!(backend.read(&name).unwrap_err().to_string().contains("503"));
    assert_eq!(worker.join().unwrap().len(), 5);
}

#[test]
fn s3_prefix_url_decoding_preserves_names_and_refuses_traversal() {
    for (prefix, expected) in [
        ("my%20repo/%E2%98%83", "my repo/☃/"),
        ("a%2Bb/%25", "a+b/%/"),
        ("a%2Fb", "a/b/"),
    ] {
        assert_eq!(
            S3Location::parse(&format!("s3:https://host/bucket/{prefix}"))
                .unwrap()
                .prefix,
            expected
        );
    }
    for prefix in [
        "%2e%2e", "a/%2E/b", "%00", "%5c", "%ff", "%", "%a", "%gg", "a\nb",
    ] {
        assert!(S3Location::parse(&format!("s3:https://host/bucket/{prefix}")).is_err());
    }
}
