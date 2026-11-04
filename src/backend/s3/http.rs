use super::{S3Backend, error};
use crate::backend::BackendError;
use rusty_s3::S3Action;
use std::{io, thread, time::Duration};

pub(super) struct Response {
    pub status: u16,
    pub size: Option<u64>,
    pub content_range: Option<String>,
    pub bytes: Vec<u8>,
}

impl S3Backend {
    // Never retain HTTP errors or response text: either can contain presigned URLs.
    pub(super) fn request<'a, A: S3Action<'a>>(
        &self,
        mut action: A,
        headers: &[(&str, &str)],
        bytes: &[u8],
    ) -> Result<Response, BackendError> {
        for (key, value) in headers {
            action
                .headers_mut()
                .insert((*key).to_owned(), (*value).to_owned());
        }
        for attempt in 0..5 {
            let url = action.sign(Duration::from_secs(3600));
            let mut request = ureq::http::Request::builder()
                .method(A::METHOD.to_string().as_str())
                .uri(url.as_str());
            for (key, value) in headers {
                request = request.header(*key, *value);
            }
            let request = request
                .body(bytes)
                .map_err(|_| error("invalid S3 request"))?;
            let result = self.agent.run(request).and_then(|mut r| {
                let status = r.status().as_u16();
                let size = r
                    .headers()
                    .get("content-length")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse().ok());
                let content_range = r
                    .headers()
                    .get("content-range")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                let bytes = if (200..300).contains(&status) {
                    r.body_mut()
                        .with_config()
                        .limit(128 * 1024 * 1024)
                        .read_to_vec()?
                } else {
                    Vec::new()
                };
                Ok(Response {
                    status,
                    size,
                    content_range,
                    bytes,
                })
            });
            let retry = match &result {
                Ok(r) => r.status == 429 || (500..600).contains(&r.status),
                Err(ureq::Error::Io(e)) => e.kind() == io::ErrorKind::ConnectionReset,
                _ => false,
            };
            if retry && attempt < 4 {
                thread::sleep(Duration::from_millis((100_u64 << attempt).min(1000)));
                continue;
            }
            return result.map_err(|e| match e {
                ureq::Error::Timeout(_) => error("S3 request timed out"),
                _ => error("S3 transport failed (check endpoint, TLS, and connectivity)"),
            });
        }
        unreachable!("bounded retry loop returns on its last attempt")
    }
}

pub(super) fn success(r: Response) -> Result<Response, BackendError> {
    if (200..300).contains(&r.status) {
        Ok(r)
    } else {
        Err(error(&format!("S3 HTTP {}", r.status)))
    }
}
