use super::{BackendError, Kind, ObjectName};

// Raw names are repository-relative, and may never resolve outside their kind.
// A valid object must go through the typed delete operation instead.
pub(super) fn validate(raw: &str) -> Result<Kind, BackendError> {
    let invalid = || BackendError::InvalidName(raw.to_owned());
    if raw.is_empty()
        || raw.starts_with('/')
        || raw.contains("..")
        || raw.contains(['\\', '\0'])
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
    match raw.split_once('/') {
        None => Some(Kind::Config),
        Some(("keys", _)) => Some(Kind::Keys),
        Some(("data", _)) => Some(Kind::Data),
        Some(("index", _)) => Some(Kind::Index),
        Some(("snapshots", _)) => Some(Kind::Snapshots),
        Some(("locks", _)) => Some(Kind::Locks),
        _ => None,
    }
}
