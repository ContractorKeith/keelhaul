//! Path-style S3, signed with rusty-s3 and sent synchronously over rustls.
mod config;
mod deadline;
mod error_code;
mod http;
mod list;
mod range;
#[cfg(test)]
mod request_tests;
mod retry;
#[cfg(test)]
mod retry_tests;
#[cfg(test)]
mod tests;
use self::http::{Response, success};
use super::{Backend, BackendError, Entry, ForeignEntry, Kind, ObjectName, foreign};
pub use config::{S3Config, S3Location};
use rusty_s3::{Bucket, Credentials};
use std::{fmt, io, sync::Mutex};

pub struct S3Backend {
    bucket: Bucket,
    credentials: Credentials,
    prefix: String,
    agent: ureq::Agent,
    timeout: std::time::Duration,
    deadline: std::time::Duration,
    conditional: Mutex<Option<bool>>,
    verbose: bool,
}

impl fmt::Debug for S3Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("S3Backend").finish_non_exhaustive()
    }
}

fn error(message: &str) -> BackendError {
    io::Error::other(message.to_owned()).into()
}

impl S3Backend {
    pub fn new(config: S3Config) -> Result<Self, BackendError> {
        config.connect()
    }

    fn key(&self, raw: &str) -> String {
        format!("{}{raw}", self.prefix)
    }

    fn object_response(&self, name: &ObjectName, r: Response) -> Result<Response, BackendError> {
        if r.status == 404 {
            Err(BackendError::NotFound(name.clone()))
        } else {
            success(r).map_err(|e| e.context(Some(name), None))
        }
    }

    // Probe an isolated temporary object, so servers silently ignoring the header
    // are detected too. If killed before DELETE, SPEC §13 lets prune remove
    // this temporary object once it is older than 30 minutes.
    fn detect_conditional(&self) -> Result<bool, BackendError> {
        let mut nonce = [0; 8];
        getrandom::fill(&mut nonce).map_err(|_| error("S3 probe nonce generation failed"))?;
        let key = self.key(&format!("config.tmp-{}", hex::encode(nonce)));
        success(self.request(
            self.bucket.put_object(Some(&self.credentials), &key),
            &[],
            b"probe",
        )?)?;
        let probe = self.request(
            self.bucket.put_object(Some(&self.credentials), &key),
            &[("if-none-match", "*")],
            b"probe",
        );
        success(self.request(
            self.bucket.delete_object(Some(&self.credentials), &key),
            &[],
            &[],
        )?)?;
        let supported = match probe?.status {
            412 => true,
            200..=299 | 400..=499 | 501 => false,
            status => return Err(error(&format!("S3 conditional-write probe HTTP {status}"))),
        };
        if self.verbose {
            crate::warning!(
                "S3 create-once: {}",
                if supported {
                    "If-None-Match: *"
                } else {
                    "HEAD-then-PUT (no cross-client exclusion)"
                }
            );
        }
        Ok(supported)
    }

    fn same(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        if self.read(name)? == bytes {
            Ok(())
        } else {
            Err(BackendError::AlreadyExists(name.clone()))
        }
    }
}

impl Backend for S3Backend {
    /// Create-once is exact only with conditional PUT; otherwise concurrent init
    /// of one repository is unsafe because HEAD-then-PUT cannot exclude writers.
    fn write(&self, name: &ObjectName, bytes: &[u8]) -> Result<(), BackendError> {
        let mut state = self
            .conditional
            .lock()
            .map_err(|_| error("S3 write mutex poisoned"))?;
        let conditional = match *state {
            Some(supported) => supported,
            None => {
                let supported = self.detect_conditional()?;
                *state = Some(supported);
                supported
            }
        };
        drop(state);
        // Content-addressed names imply identical bytes for racing writers.
        // Config and key slots are initialized once, so HEAD-then-PUT is acceptable
        // without serializing uploads when conditional PUT is unavailable.
        let key = self.key(name.as_str());
        if !conditional {
            match self.stat(name) {
                Ok(_) => return self.same(name, bytes),
                Err(BackendError::NotFound(_)) => {}
                Err(e) => return Err(e),
            }
        }
        let headers = if conditional {
            &[("if-none-match", "*")][..]
        } else {
            &[]
        };
        let r = self.request(
            self.bucket.put_object(Some(&self.credentials), &key),
            headers,
            bytes,
        )?;
        if r.status == 412 {
            self.same(name, bytes)
        } else {
            self.object_response(name, r).map(|_| ())
        }
    }

    fn read(&self, name: &ObjectName) -> Result<Vec<u8>, BackendError> {
        let key = self.key(name.as_str());
        self.object_response(
            name,
            self.request(
                self.bucket.get_object(Some(&self.credentials), &key),
                &[],
                &[],
            )?,
        )
        .map(|r| r.bytes)
    }

    fn read_range(
        &self,
        name: &ObjectName,
        offset: u64,
        len: u64,
    ) -> Result<Vec<u8>, BackendError> {
        if len == 0 {
            return Ok(Vec::new());
        }
        let end = offset.saturating_add(len - 1);
        let range = format!("bytes={offset}-{end}");
        let key = self.key(name.as_str());
        let r = self.request(
            self.bucket.get_object(Some(&self.credentials), &key),
            &[("range", &range)],
            &[],
        )?;
        if r.status == 416 {
            return Ok(Vec::new());
        }
        range::bytes(self.object_response(name, r)?, offset, end)
    }

    fn list(&self, kind: Kind) -> Result<Vec<Entry>, BackendError> {
        Ok(self
            .scan(kind)?
            .into_iter()
            .filter_map(|e| {
                let name = ObjectName::new(&e.raw).ok()?;
                (name.kind() == kind).then_some(Entry {
                    name,
                    size: e.size,
                    modified: e.modified,
                })
            })
            .collect())
    }

    fn list_foreign(&self, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        Ok(self
            .scan(kind)?
            .into_iter()
            .filter(|e| foreign::kind(&e.raw) == Some(kind))
            .collect())
    }

    fn delete_foreign(&self, raw: &str) -> Result<(), BackendError> {
        foreign::validate(raw)?;
        let key = self.key(raw);
        let r = self.request(
            self.bucket.delete_object(Some(&self.credentials), &key),
            &[],
            &[],
        )?;
        if r.status == 404 {
            Ok(())
        } else {
            success(r).map(|_| ())
        }
    }

    fn stat(&self, name: &ObjectName) -> Result<u64, BackendError> {
        let key = self.key(name.as_str());
        self.object_response(
            name,
            self.request(
                self.bucket.head_object(Some(&self.credentials), &key),
                &[],
                &[],
            )?,
        )?
        .size
        .ok_or_else(|| error("S3 HEAD omitted content length"))
    }

    fn delete(&self, name: &ObjectName) -> Result<(), BackendError> {
        let key = self.key(name.as_str());
        let r = self.request(
            self.bucket.delete_object(Some(&self.credentials), &key),
            &[],
            &[],
        )?;
        if r.status == 404 {
            Ok(())
        } else {
            self.object_response(name, r).map(|_| ())
        }
    }
}
