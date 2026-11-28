use super::{BackendError, Kind, ObjectName};

pub(super) fn validate(raw: &str) -> Result<Kind, BackendError> {
    let invalid = || BackendError::InvalidName(raw.to_owned());
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.contains("..")
        || raw.contains(['\\', '\0', '\u{fffd}'])
        || ObjectName::new(raw).is_ok()
    {
        return Err(invalid());
    }
    let parts: Vec<_> = raw.split('/').collect();
    if matches!(parts.as_slice(), ["data", _]) {
        return Err(invalid());
    }
    if parts.iter().any(|part| part.is_empty() || *part == ".") {
        return Err(invalid());
    }
    kind(raw).ok_or_else(invalid)
}

pub(super) fn kind(raw: &str) -> Option<Kind> {
    if ObjectName::new(raw).is_ok() {
        return None;
    }
    position(raw)
}

pub(super) fn position(raw: &str) -> Option<Kind> {
    let parts: Vec<_> = raw.split('/').collect();
    match parts.as_slice() {
        [_] => Some(Kind::Config),
        ["keys", _] => Some(Kind::Keys),
        ["index", _] => Some(Kind::Index),
        ["snapshots", _] => Some(Kind::Snapshots),
        ["locks", _] => Some(Kind::Locks),
        ["data", _] => Some(Kind::Data),
        ["data", shard, _] if super::valid_hex(shard, 2) => Some(Kind::Data),
        _ => None,
    }
}
