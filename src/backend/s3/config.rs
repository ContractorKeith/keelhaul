use super::{S3Backend, error};
use crate::backend::BackendError;
use rusty_s3::{Bucket, Credentials, UrlStyle};
use std::sync::Mutex;
use std::{env, fmt, time::Duration};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3Location {
    pub(super) endpoint: String,
    pub(super) bucket: String,
    pub(super) prefix: String,
}

impl S3Location {
    pub fn parse(repo: &str) -> Result<Self, BackendError> {
        let invalid = || error("expected s3:https://endpoint/bucket[/prefix]");
        let rest = repo.strip_prefix("s3:").ok_or_else(invalid)?;
        // Use the same URL parser as the signer, without adding a second URL dependency.
        let parsed = Bucket::new(
            rest.parse().map_err(|_| invalid())?,
            UrlStyle::Path,
            "probe",
            "us-east-1",
        )
        .map_err(|_| invalid())?;
        let url = parsed.base_url();
        // Check the original authority too: Bucket deliberately strips userinfo/query.
        if rest.contains(['?', '#', '\\'])
            || rest.split('/').nth(2).is_some_and(|s| s.contains('@'))
        {
            return Err(invalid());
        }
        let path = rest
            .split_once("://")
            .ok_or_else(invalid)?
            .1
            .split_once('/')
            .ok_or_else(invalid)?
            .1;
        let mut parts = path.split('/');
        let bucket = parts.next().ok_or_else(invalid)?;
        if !(3..=63).contains(&bucket.len())
            || !bucket
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b".-".contains(&b))
            || !bucket.as_bytes()[0].is_ascii_alphanumeric()
            || !bucket.as_bytes()[bucket.len() - 1].is_ascii_alphanumeric()
            || bucket.contains("..")
            || bucket.parse::<std::net::Ipv4Addr>().is_ok()
        {
            return Err(invalid());
        }
        let prefix = parts.collect::<Vec<_>>().join("/");
        let mut decoded = Vec::new();
        let mut bytes = prefix.bytes();
        while let Some(byte) = bytes.next() {
            if byte == b'%' {
                let pair = [
                    bytes.next().ok_or_else(invalid)?,
                    bytes.next().ok_or_else(invalid)?,
                ];
                let mut value = [0];
                hex::decode_to_slice(pair, &mut value).map_err(|_| invalid())?;
                decoded.push(value[0]);
            } else {
                decoded.push(byte);
            }
        }
        let prefix = String::from_utf8(decoded).map_err(|_| invalid())?;
        if prefix.starts_with('/') {
            return Err(invalid());
        }
        let prefix = prefix.strip_suffix('/').unwrap_or(&prefix);
        if (!prefix.is_empty() && prefix.split('/').any(|p| matches!(p, "" | "." | "..")))
            || prefix.chars().any(char::is_control)
            || prefix.contains('\\')
        {
            return Err(invalid());
        }
        Ok(Self {
            endpoint: url.origin().ascii_serialization(),
            bucket: bucket.to_owned(),
            prefix: if prefix.is_empty() {
                String::new()
            } else {
                format!("{prefix}/")
            },
        })
    }

    pub(super) fn bucket(&self, region: &str) -> Result<Bucket, BackendError> {
        Bucket::new(
            self.endpoint
                .parse()
                .map_err(|_| error("invalid S3 endpoint"))?,
            UrlStyle::Path,
            self.bucket.clone(),
            region.to_owned(),
        )
        .map_err(|_| error("invalid S3 endpoint"))
    }
}

pub struct S3Config {
    pub location: S3Location,
    credentials: Credentials,
    pub region: String,
    /// Per-attempt timeout (`KEELHAUL_S3_TIMEOUT_SECS`, default 60 seconds).
    /// Up to five attempts run, with capped, jittered backoff between attempts.
    pub timeout: Duration,
    pub verbose: bool,
}

impl S3Config {
    /// Credentials remain private and Debug always redacts them.
    /// ```compile_fail
    /// use keelhaul::backend::s3::{S3Config, S3Location};
    /// let config = S3Config::new(S3Location::parse("s3:https://host/bucket").unwrap(),
    ///     rusty_s3::Credentials::new("access", "secret"));
    /// println!("{:?}", config.credentials); // private: use the redacted config Debug
    /// ```
    pub fn new(location: S3Location, credentials: Credentials) -> Self {
        Self {
            location,
            credentials,
            region: "us-east-1".into(),
            timeout: Duration::from_secs(60),
            verbose: false,
        }
    }

    pub(super) fn connect(self) -> Result<S3Backend, BackendError> {
        let bucket = self.location.bucket(&self.region)?;
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(self.timeout))
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .new_agent();
        Ok(S3Backend {
            bucket,
            credentials: self.credentials,
            prefix: self.location.prefix,
            agent,
            conditional: Mutex::new(None),
            verbose: self.verbose,
        })
    }

    pub fn from_env(location: S3Location, verbose: bool) -> Result<Self, BackendError> {
        let credentials = Credentials::from_env()
            .ok_or_else(|| error("S3 requires AWS_ACCESS_KEY_ID and AWS_SECRET_ACCESS_KEY"))?;
        let timeout = match env::var("KEELHAUL_S3_TIMEOUT_SECS") {
            Ok(value) => value
                .parse::<u64>()
                .ok()
                .filter(|n| *n > 0)
                .ok_or_else(|| error("KEELHAUL_S3_TIMEOUT_SECS must be a positive integer"))?,
            Err(env::VarError::NotPresent) => 60,
            Err(_) => return Err(error("invalid KEELHAUL_S3_TIMEOUT_SECS")),
        };
        Ok(Self {
            location,
            credentials,
            region: env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".into()),
            timeout: Duration::from_secs(timeout),
            verbose,
        })
    }
}

impl fmt::Debug for S3Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let suffix: String = self
            .credentials
            .key()
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        f.debug_struct("S3Config")
            .field("access_key", &format!("****{suffix}"))
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}
