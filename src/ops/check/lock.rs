//! Shared lock payload validation.
pub(super) fn parse(bytes: &[u8]) -> Result<(), String> {
    crate::repo::lock::Lock::parse(bytes).map(|_| ())
}
