use super::{tests::mock_headers, *};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Barrier, Condvar, Mutex},
    thread,
    time::Duration,
};

#[test]
fn s3_range_responses_need_one_get_and_validate_clipping() {
    let name = ObjectName::new("config").unwrap();
    let (backend, worker) = mock_headers(vec![
        (206, "ef", "Content-Range: bytes 4-5/6\r\n"),
        (200, "abcdef", ""),
        (200, "abcdef", ""),
        (416, "", ""),
        (206, "e", "Content-Range: bytes 4-5/6\r\n"),
        (206, "ef", ""),
        (206, "ef", "Content-Range: bytes 3-4/6\r\n"),
    ]);
    assert_eq!(backend.read_range(&name, 4, 20).unwrap(), b"ef");
    assert_eq!(backend.read_range(&name, 2, u64::MAX).unwrap(), b"cdef");
    assert_eq!(backend.read_range(&name, 2, 2).unwrap(), b"cd");
    assert!(backend.read_range(&name, u64::MAX, 1).unwrap().is_empty());
    for _ in 0..3 {
        assert!(backend.read_range(&name, 4, 20).is_err());
    }
    // Even a missing name needs no request for an empty range.
    assert!(backend.read_range(&name, 0, 0).unwrap().is_empty());
    let requests = worker.join().unwrap();
    assert_eq!(requests.len(), 7);
    assert!(requests.iter().all(|r| r.starts_with("GET ")));
    assert!(requests[0].to_lowercase().contains("range: bytes=4-23\r\n"));
    assert!(
        requests[1]
            .to_lowercase()
            .contains(&format!("range: bytes=2-{}\r\n", u64::MAX))
    );
}

#[test]
fn s3_fallback_eight_distinct_uploads_overlap_after_one_probe() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("s3:http://{}/bucket/repo", listener.local_addr().unwrap());
    let overlap = Arc::new((Mutex::new((0_usize, 0_usize)), Condvar::new()));
    let observed = overlap.clone();
    // Three probe requests followed by eight HEAD/PUT pairs. Every accepted
    // connection has its own worker so a backend lock is observable on the wire.
    let server = thread::spawn(move || {
        thread::scope(|scope| {
            let mut handlers = Vec::new();
            for _ in 0..19 {
                let (mut stream, _) = listener.accept().unwrap();
                let overlap = &observed;
                handlers.push(scope.spawn(move || {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(3)))
                        .unwrap();
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        stream.read_exact(&mut byte).unwrap();
                        header.push(byte[0]);
                    }
                    let header = String::from_utf8(header).unwrap();
                    let length: usize = header
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse().unwrap())
                        })
                        .unwrap_or(0);
                    stream.read_exact(&mut vec![0; length]).unwrap();
                    let probe = header.lines().next().unwrap().contains(".tmp-");
                    let upload = header.starts_with("PUT ") && !probe;
                    if upload {
                        let (lock, ready) = &**overlap;
                        let mut state = lock.lock().unwrap();
                        state.0 += 1;
                        state.1 = state.1.max(state.0);
                        ready.notify_all();
                        // A serialized implementation times out with peak == 1,
                        // instead of deadlocking the test forever.
                        let (mut state, _) = ready
                            .wait_timeout_while(state, Duration::from_secs(1), |s| s.1 < 2)
                            .unwrap();
                        state.0 -= 1;
                    }
                    let status = if header.starts_with("HEAD ") {
                        404
                    } else {
                        200
                    };
                    write!(
                        stream,
                        "HTTP/1.1 {status} Mock\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                    probe && header.to_lowercase().contains("if-none-match:")
                }));
            }
            handlers
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(|probe| *probe)
                .count()
        })
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
    let start = Barrier::new(8);
    thread::scope(|scope| {
        let writers: Vec<_> = (0..8)
            .map(|id| {
                let backend = &backend;
                let start = &start;
                scope.spawn(move || {
                    start.wait();
                    backend
                        .write(
                            &ObjectName::new(&format!("index/{id:064x}")).unwrap(),
                            b"object",
                        )
                        .unwrap();
                })
            })
            .collect();
        for writer in writers {
            writer.join().unwrap();
        }
    });
    assert_eq!(server.join().unwrap(), 1, "conditional support probed once");
    assert!(
        overlap.0.lock().unwrap().1 > 1,
        "fallback PUTs must overlap"
    );
}
