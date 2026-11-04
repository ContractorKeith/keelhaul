use super::{BackendError, Kind, ObjectName};

// Raw names are repository-relative, and may never resolve outside their kind.
// A valid object must go through the typed delete operation instead.
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

// Only object-depth entries belong to the namespace; never descend arbitrarily.
pub(super) fn position(raw: &str) -> Option<Kind> {
    let parts: Vec<_> = raw.split('/').collect();
    match parts.as_slice() {
        [_] => Some(Kind::Config),
        ["keys", _] => Some(Kind::Keys),
        ["index", _] => Some(Kind::Index),
        ["snapshots", _] => Some(Kind::Snapshots),
        ["locks", _] => Some(Kind::Locks),
        ["data", shard, _] if super::valid_hex(shard, 2) => Some(Kind::Data),
        _ => None,
    }
}
