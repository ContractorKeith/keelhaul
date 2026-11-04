use crate::repo::Error;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
pub const STALE: Duration = Duration::from_secs(30 * 60);
pub trait Clock: Sync {
    fn now(&self) -> SystemTime;
}
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Lock {
    pub version: u32,
    pub exclusive: bool,
    pub host: String,
    pub pid: u32,
    pub username: String,
    pub created: String,
    pub refreshed: String,
}
impl Lock {
    pub fn new(exclusive: bool, now: SystemTime) -> Result<Self, Error> {
        let now = timestamp(now)?;
        Ok(Self {
            version: 1,
            exclusive,
            host: crate::host::hostname(),
            pid: std::process::id(),
            username: crate::host::username(),
            created: now.clone(),
            refreshed: now,
        })
    }
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let lock: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if lock.version != 1 {
            return Err("unknown lock version".into());
        }
        crate::format::validate_time(&lock.created)?;
        crate::format::validate_time(&lock.refreshed)?;
        Ok(lock)
    }
    pub fn age(&self, now: SystemTime) -> Result<Duration, Error> {
        let refreshed = time::OffsetDateTime::parse(
            &self.refreshed,
            &time::format_description::well_known::Rfc3339,
        )
        .map_err(|e| Error::Refused(e.to_string()))?;
        Ok(now
            .duration_since(SystemTime::from(refreshed))
            .unwrap_or_default())
    }
    pub fn stale(&self, now: SystemTime) -> Result<bool, Error> {
        if self.age(now)? > STALE {
            return Ok(true);
        }
        if self.host != crate::host::hostname() {
            return Ok(false);
        }
        let Some(pid) = rustix::process::Pid::from_raw(self.pid.try_into().unwrap_or(0)) else {
            return Ok(true);
        };
        // EPERM means the PID exists; only ESRCH proves it is gone.
        Ok(matches!(
            rustix::process::test_kill_process(pid),
            Err(rustix::io::Errno::SRCH)
        ))
    }
}
pub(super) fn timestamp(now: SystemTime) -> Result<String, Error> {
    time::OffsetDateTime::from(now)
        .format(
            &time::format_description::parse_borrowed::<2>(
                "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:9]Z",
            )
            .expect("fixed UTC format"),
        )
        .map_err(|e| Error::Refused(e.to_string()))
}
