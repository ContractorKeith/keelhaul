use super::{S3Backend, error, retry};
use crate::backend::BackendError;
use rusty_s3::S3Action;
use std::{thread, time::Duration};

pub(super) struct Response {
    pub status: u16,
    pub size: Option<u64>,
    retry_after: Option<u64>,
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
        for attempt in 0..retry::ATTEMPTS {
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
                let retry_after = if matches!(status, 429 | 503) {
                    r.headers()
                        .get("retry-after")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.trim().parse::<u64>().ok())
                } else {
                    None
                };
                let bytes = if (200..300).contains(&status) {
                    if A::METHOD != rusty_s3::Method::Head
                        && size.is_some_and(|size| size > retry::BODY_LIMIT)
                    {
                        return Err(ureq::Error::BodyExceedsLimit(retry::BODY_LIMIT));
                    }
                    r.body_mut()
                        .with_config()
                        .limit(retry::BODY_LIMIT)
                        .read_to_vec()?
                } else {
                    Vec::new()
                };
                Ok(Response {
                    status,
                    size,
                    content_range,
                    retry_after,
                    bytes,
                })
            });
            let retry = match &result {
                Ok(r) => r.status == 429 || (500..600).contains(&r.status),
                Err(e) => retry::retryable(e),
            };
            if retry && attempt + 1 < retry::ATTEMPTS {
                let retry_after = result.as_ref().ok().and_then(|r| r.retry_after);
                let delay = retry::delay(attempt, retry_after)?;
                if self.verbose {
                    match &result {
                        Err(e) => {
                            eprintln!("{}; retrying in {} ms", retry::cause(e), delay.as_millis())
                        }
                        Ok(r) => {
                            eprintln!("S3 HTTP {}; retrying in {} ms", r.status, delay.as_millis())
                        }
                    }
                }
                thread::sleep(delay);
                continue;
            }
            return result.map_err(|e| error(retry::cause(&e)));
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
