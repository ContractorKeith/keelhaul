use super::{S3Backend, error, http::success};
use crate::backend::{BackendError, ForeignEntry, Kind, foreign};
use rusty_s3::actions::ListObjectsV2;
use std::collections::HashSet;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

impl S3Backend {
    pub(super) fn scan(&self, kind: Kind) -> Result<Vec<ForeignEntry>, BackendError> {
        let prefix = if kind == Kind::Config {
            self.prefix.clone()
        } else {
            format!("{}{}/", self.prefix, kind.directory())
        };
        let mut token = None;
        let mut seen = HashSet::new();
        let mut entries = Vec::new();
        loop {
            let mut action = self.bucket.list_objects_v2(Some(&self.credentials));
            action.with_prefix(&prefix);
            if kind != Kind::Data {
                action.with_delimiter("/");
            }
            if let Some(token) = &token {
                action.with_continuation_token(token);
            }
            let response = success(self.request(action, &[], &[])?)?;
            let xml = std::str::from_utf8(&response.bytes)
                .map_err(|_| error("invalid S3 listing UTF-8"))?;
            let page =
                ListObjectsV2::parse_response(xml).map_err(|_| error("invalid S3 listing XML"))?;
            for object in page.contents {
                let raw = object
                    .key
                    .strip_prefix(&self.prefix)
                    .ok_or_else(|| error("S3 listing escaped repository prefix"))?;
                if foreign::position(raw) != Some(kind) {
                    continue;
                }
                let modified = object
                    .last_modified
                    .as_deref()
                    .map(|s| OffsetDateTime::parse(s, &Rfc3339).map(Into::into))
                    .transpose()
                    .map_err(|_| error("invalid S3 modification time"))?;
                entries.push(ForeignEntry {
                    raw: raw.to_owned(),
                    deletable: foreign::validate(raw).is_ok(),
                    size: object
                        .size
                        .ok_or_else(|| error("S3 listing omitted object size"))?,
                    modified,
                });
            }
            token = page.next_continuation_token;
            match &token {
                None => break,
                Some(t) if !seen.insert(t.clone()) => {
                    return Err(error("S3 listing repeated continuation token"));
                }
                _ => {}
            }
        }
        entries.sort_by(|a, b| a.raw.cmp(&b.raw));
        Ok(entries)
    }
}
