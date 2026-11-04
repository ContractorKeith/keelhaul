use super::{BackendError, error};
use std::{io, time::Duration};

pub(super) const ATTEMPTS: u32 = 5;
pub(super) const BODY_LIMIT: u64 = 128 * 1024 * 1024;
const MAX_BACKOFF_MS: u64 = 1000;

pub(super) fn retryable(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::Io(e) => matches!(
            e.kind(),
            io::ErrorKind::ConnectionReset
                | io::ErrorKind::BrokenPipe
                | io::ErrorKind::UnexpectedEof
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::TimedOut
        ),
        ureq::Error::Timeout(_) => true,
        _ => false,
    }
}

// Keep all messages independent of URLs, credentials and server-provided text.
pub(super) fn cause(error: &ureq::Error) -> &'static str {
    match error {
        ureq::Error::BodyExceedsLimit(_) => "S3 response body exceeds 128 MiB",
        ureq::Error::Timeout(_) => "S3 request timed out (per-attempt timeout; up to 5 attempts)",
        ureq::Error::Io(e) => match e.kind() {
            io::ErrorKind::ConnectionReset => "S3 transport failed: connection reset",
            io::ErrorKind::BrokenPipe => "S3 transport failed: broken pipe",
            io::ErrorKind::UnexpectedEof => "S3 transport failed: unexpected EOF",
            io::ErrorKind::ConnectionAborted => "S3 transport failed: connection aborted",
            io::ErrorKind::TimedOut => "S3 transport failed: timed out",
            _ => "S3 transport failed (check endpoint, TLS, and connectivity)",
        },
        _ => "S3 transport failed (check endpoint, TLS, and connectivity)",
    }
}

pub(super) fn delay(attempt: u32, retry_after: Option<u64>) -> Result<Duration, BackendError> {
    let mut random = [0; 8];
    getrandom::fill(&mut random).map_err(|_| error("S3 retry jitter generation failed"))?;
    Ok(backoff(attempt, retry_after, u64::from_le_bytes(random)))
}

fn backoff(attempt: u32, retry_after: Option<u64>, random: u64) -> Duration {
    let cap = (100_u64 << attempt).min(MAX_BACKOFF_MS);
    let jittered = cap / 2 + random % (cap - cap / 2 + 1);
    let minimum = retry_after
        .unwrap_or(0)
        .saturating_mul(1000)
        .min(MAX_BACKOFF_MS);
    Duration::from_millis(jittered.max(minimum))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn s3_retry_after_is_honored_and_capped_with_jitter() {
        for attempt in 0..ATTEMPTS {
            assert_eq!(backoff(attempt, Some(u64::MAX), 0), Duration::from_secs(1));
            assert_eq!(backoff(attempt, Some(1), u64::MAX), Duration::from_secs(1));
            let low = backoff(attempt, None, 0);
            let high = backoff(attempt, None, 1);
            assert!(high > low && high <= Duration::from_secs(1));
        }
    }
    #[test]
    fn s3_retry_causes_are_named_without_io_error_secrets() {
        for (kind, name) in [
            (io::ErrorKind::ConnectionReset, "connection reset"),
            (io::ErrorKind::BrokenPipe, "broken pipe"),
            (io::ErrorKind::UnexpectedEof, "unexpected EOF"),
            (io::ErrorKind::ConnectionAborted, "connection aborted"),
            (io::ErrorKind::TimedOut, "timed out"),
        ] {
            let e = ureq::Error::Io(io::Error::new(kind, "secret-signed-url"));
            assert!(retryable(&e));
            assert!(cause(&e).contains(name));
            assert!(!cause(&e).contains("secret"));
        }
        assert!(!retryable(&ureq::Error::BodyExceedsLimit(BODY_LIMIT)));
        assert_eq!(
            cause(&ureq::Error::BodyExceedsLimit(BODY_LIMIT)),
            "S3 response body exceeds 128 MiB"
        );
    }
}
