use super::deadline;
use super::{S3Backend, error, error_code, retry};
use crate::backend::BackendError;
use rusty_s3::S3Action;
use std::time::Duration;
use ureq::RequestExt;

pub(super) struct Response {
    pub status: u16,
    pub size: Option<u64>,
    retry_after: Option<u64>,
    missing_bucket: bool,
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
        let clock = deadline::SystemClock::default();
        let result = deadline::run(self.deadline, self.timeout, &clock, |attempt, timeout| {
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
            let result = request
                .with_agent(&self.agent)
                .configure()
                .timeout_global(Some(timeout))
                .run()
                .and_then(|mut r| {
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
                    let missing_bucket = if status == 404 {
                        let body = r
                            .body_mut()
                            .with_config()
                            .limit(retry::BODY_LIMIT)
                            .read_to_vec()?;
                        error_code::is_missing_bucket(&body)
                    } else {
                        false
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
                        missing_bucket,
                        bytes,
                    })
                });
            if result.as_ref().is_ok_and(|r| r.missing_bucket) {
                return Err(BackendError::BucketNotFound(self.bucket.name().to_owned()));
            }
            let retry = match &result {
                Ok(r) => r.status == 429 || (500..600).contains(&r.status),
                Err(e) => retry::retryable(e),
            };
            if retry && attempt + 1 < retry::ATTEMPTS && self.verbose {
                match &result {
                    Err(e) => crate::warning!("{}; retrying", retry::cause(e)),
                    Ok(r) => crate::warning!("S3 HTTP {}; retrying", r.status),
                }
            }
            let retry_after = result.as_ref().ok().and_then(|r| r.retry_after);
            Ok(deadline::Attempt {
                result,
                retry,
                retry_after,
            })
        })?;
        result.map_err(|e| error(retry::cause(&e)))
    }
}

pub(super) fn success(r: Response) -> Result<Response, BackendError> {
    if (200..300).contains(&r.status) {
        Ok(r)
    } else {
        Err(error(&format!("S3 HTTP {}", r.status)))
    }
}
