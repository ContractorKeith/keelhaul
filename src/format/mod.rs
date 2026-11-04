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
