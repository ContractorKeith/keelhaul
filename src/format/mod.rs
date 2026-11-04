pub mod config;
pub mod key_slot;

pub(crate) fn timestamp() -> String {
    time::OffsetDateTime::now_utc()
        .format(
            &time::format_description::parse_borrowed::<2>(
                "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:9]Z",
            )
            .expect("fixed timestamp format is valid"),
        )
        .expect("UTC date fits RFC 3339")
}

pub(crate) fn validate_time(value: &str) -> Result<(), String> {
    let bytes = value.as_bytes();
    if bytes.len() != 30
        || bytes[10] != b'T'
        || bytes[19] != b'.'
        || bytes[29] != b'Z'
        || !bytes[20..29].iter().all(u8::is_ascii_digit)
    {
        return Err("timestamp must use UTC Z and exactly 9 fractional digits".into());
    }
    time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339)
        .map(|_| ())
        .map_err(|e| format!("invalid timestamp: {e}"))
}

pub(crate) fn valid_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub mod index;
pub mod pack;
pub mod snapshot;
pub mod tree;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ParseError(pub String);
impl From<String> for ParseError {
    fn from(value: String) -> Self {
        Self(value)
    }
}
impl From<serde_json::Error> for ParseError {
    fn from(value: serde_json::Error) -> Self {
        Self(value.to_string())
    }
}
pub(crate) fn require(ok: bool, message: &str) -> Result<(), ParseError> {
    if ok {
        Ok(())
    } else {
        Err(ParseError(message.into()))
    }
}
