//! Lock payload validation; acquisition and stale-lock policy arrive in M5.
use serde::Deserialize;
#[derive(Deserialize)]
struct Lock {
    version: u32,
    exclusive: bool,
    host: String,
    pid: u32,
    username: String,
    created: String,
    refreshed: String,
}
pub(super) fn parse(bytes: &[u8]) -> Result<(), String> {
    let lock: Lock = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    if lock.version != 1 {
        return Err("unknown lock version".into());
    }
    crate::format::validate_time(&lock.created)?;
    crate::format::validate_time(&lock.refreshed)?;
    let _ = (lock.exclusive, lock.host, lock.pid, lock.username);
    Ok(())
}
