//! A tiny HTTP/1.1 fault proxy; preserves the signed Host header.
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct Proxy {
    pub endpoint: String,
    pub requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Proxy {
    pub fn start(failures: usize, status: u16) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let count = requests.clone();
        let stopping = stop.clone();
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                let (mut client, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(e) => panic!("proxy accept: {e}"),
                };
                client
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut data = Vec::new();
                let header_end = loop {
                    let mut byte = [0];
                    client.read_exact(&mut byte).unwrap();
                    data.push(byte[0]);
                    if data.ends_with(b"\r\n\r\n") {
                        break data.len();
                    }
                    assert!(data.len() < 32768);
                };
                let header = String::from_utf8(data[..header_end].to_vec()).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.trim().parse().unwrap())
                    })
                    .unwrap_or(0);
                data.resize(header_end + length, 0);
                client.read_exact(&mut data[header_end..]).unwrap();
                if count.fetch_add(1, Ordering::AcqRel) < failures {
                    write!(
                        client,
                        "HTTP/1.1 {status} Fault\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                    .unwrap();
                    continue;
                }
                let mut upstream = TcpStream::connect("127.0.0.1:7070").unwrap();
                upstream
                    .set_read_timeout(Some(Duration::from_secs(10)))
                    .unwrap();
                let header = header
                    .lines()
                    .filter(|line| !line.to_ascii_lowercase().starts_with("connection:"))
                    .collect::<Vec<_>>()
                    .join("\r\n");
                upstream
                    .write_all(header.trim_end_matches("\r\n").as_bytes())
                    .unwrap();
                upstream
                    .write_all(b"\r\nConnection: close\r\n\r\n")
                    .unwrap();
                upstream.write_all(&data[header_end..]).unwrap();
                std::io::copy(&mut upstream, &mut client).unwrap();
            }
        });
        Self {
            endpoint,
            requests,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}
