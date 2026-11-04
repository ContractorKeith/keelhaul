use crate::{
    format::tree::{Entry, Node, encode_bytes},
    repo::Error,
};
use std::{fs, os::unix::fs::MetadataExt};

pub fn timestamp(seconds: i64, nanos: i64) -> Result<String, Error> {
    let time = time::OffsetDateTime::from_unix_timestamp_nanos(
        seconds as i128 * 1_000_000_000 + nanos as i128,
    )
    .map_err(|e| Error::Refused(e.to_string()))?;
    time.format(
        &time::format_description::parse_borrowed::<2>(
            "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:9]Z",
        )
        .expect("fixed format"),
    )
    .map_err(|e| Error::Refused(e.to_string()))
}
pub fn entry(name: &[u8], meta: &fs::Metadata, node: Node) -> Result<Entry, Error> {
    let (name, name_b64) = encode_bytes(name);
    Ok(Entry {
        name,
        name_b64,
        mode: meta.mode() & 0o7777,
        mtime: timestamp(meta.mtime(), meta.mtime_nsec())?,
        ctime: timestamp(meta.ctime(), meta.ctime_nsec())?,
        uid: meta.uid(),
        gid: meta.gid(),
        user: uzers::get_user_by_uid(meta.uid()).map(|u| u.name().to_string_lossy().into_owned()),
        group: uzers::get_group_by_gid(meta.gid()).map(|g| g.name().to_string_lossy().into_owned()),
        inode: meta.ino(),
        device: meta.dev(),
        links: meta.nlink(),
        node,
    })
}
