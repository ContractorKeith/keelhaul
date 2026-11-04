use super::{tests::mock_headers, *};
use std::{
    io::{Read, Write},
    net::{Shutdown, TcpListener},
    thread,
    time::{Duration, Instant},
};

#[test]
fn s3_dropped_connections_and_truncated_bodies_retry_then_succeed() {
    for truncated in [false, true] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let location = S3Location::parse(&format!(
            "s3:http://{}/bucket",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let server = thread::spawn(move || {
            for attempt in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                if attempt < 2 && !truncated {
                    stream.shutdown(Shutdown::Both).unwrap();
                    continue;
                }
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                }
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nConnection: close\r\n\r\n")
                    .unwrap();
                stream
                    .write_all(if attempt < 2 { b"va" } else { b"value" })
                    .unwrap();
            }
            3
        });
        let backend = S3Backend::new(S3Config::new(
            location,
            Credentials::new("access", "secret"),
        ))
        .unwrap();
        assert_eq!(
            backend.read(&ObjectName::new("config").unwrap()).unwrap(),
            b"value"
        );
        assert_eq!(server.join().unwrap(), 3);
    }
}

#[test]
fn s3_retry_after_seconds_from_429_and_503_are_honored() {
    for status in [429, 503] {
        let (backend, server) = mock_headers(vec![
            (status, "", "Retry-After: 999\r\n"),
            (200, "value", ""),
        ]);
        let start = Instant::now();
        assert_eq!(
            backend.read(&ObjectName::new("config").unwrap()).unwrap(),
            b"value"
        );
        assert!(start.elapsed() >= Duration::from_secs(1));
        assert_eq!(server.join().unwrap().len(), 2);
    }
}

#[test]
fn s3_body_cap_has_an_explicit_nonretryable_error() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let location = S3Location::parse(&format!(
        "s3:http://{}/bucket",
        listener.local_addr().unwrap()
    ))
    .unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        // Headers alone exercise the real limit without a >100 MiB fixture.
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            retry::BODY_LIMIT + 1
        )
        .unwrap();
    });
    let backend = S3Backend::new(S3Config::new(
        location,
        Credentials::new("access", "secret"),
    ))
    .unwrap();
    let error = backend
        .read(&ObjectName::new("config").unwrap())
        .unwrap_err();
    assert!(
        error
            .to_string()
            .contains("S3 response body exceeds 128 MiB")
    );
    server.join().unwrap();
}

#[test]
fn s3_bucket_and_prefix_validation() {
    for bucket in ["abc", "a.b-c", "123", &"a".repeat(63)] {
        assert!(S3Location::parse(&format!("s3:https://host/{bucket}/p")).is_ok());
    }
    for bucket in [
        "ab",
        "UPPER",
        "under_score",
        "-abc",
        "abc-",
        ".abc",
        "abc.",
        "a..b",
        "127.0.0.1",
        &"a".repeat(64),
    ] {
        assert!(S3Location::parse(&format!("s3:https://host/{bucket}/p")).is_err());
    }
    for path in [
        "bucket/a//b",
        "bucket//a",
        "bucket/a//",
        "bucket//",
        "bucket/a/%2F/b",
        "s3/bucket/p",
    ] {
        assert!(
            S3Location::parse(&format!("s3:https://host/{path}")).is_err(),
            "{path}"
        );
    }
    let location = S3Location::parse("s3:https://host/proxy/bucket/p").unwrap();
    assert_eq!(location.bucket, "proxy");
    assert_eq!(location.prefix, "bucket/p/");
    assert_eq!(location.endpoint, "https://host");
}
