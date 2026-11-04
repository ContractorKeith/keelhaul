use super::{
    destination::{Destination, FileIdentity},
    plan::Item,
};
use crate::{
    backend::Backend,
    format::{pack::BlobKind, tree::Node},
    repo::{Error, blobs::BlobStore},
};
use std::{collections::HashMap, io::Write, path::PathBuf};
pub(super) struct Hardlink {
    path: PathBuf,
    identity: FileIdentity,
    size: u64,
    content: Vec<String>,
}
pub(super) type Hardlinks = HashMap<(u64, u64), Hardlink>;
pub(super) fn restore_file<B: Backend>(
    destination: &Destination,
    store: &mut BlobStore<'_, B>,
    item: &Item,
    hardlinks: &mut Hardlinks,
) -> Result<(), Error> {
    let Node::File { size, content } = &item.entry.node else {
        return Err(Error::Refused("expected a regular file".into()));
    };
    item.entry
        .validate_content_lengths(|id| {
            store
                .raw_length(BlobKind::Data, id)
                .map_err(|e| crate::format::ParseError(e.to_string()))
        })
        .map_err(|e| crate::ops::listing::damage(&item.object, &e.to_string()))?;
    let key = (item.entry.device, item.entry.inode);
    if item.entry.links > 1
        && let Some(first) = hardlinks.get(&key)
    {
        if first.size != *size || first.content != *content {
            return Err(crate::ops::listing::damage(
                &item.object,
                "inconsistent hardlink content",
            ));
        }
        if destination.hardlink(&first.path, &item.path, first.identity)? {
            return Ok(());
        }
        eprintln!(
            "warning: hardlink source {} was replaced or is unavailable; restoring {} independently",
            first.path.display(),
            item.path.display()
        );
    }
    let mut temp = destination.temporary(&item.path)?;
    for id in content {
        temp.file.write_all(&store.read(BlobKind::Data, id)?)?;
    }
    let identity = temp.publish(&item.entry)?;
    if item.entry.links > 1 {
        hardlinks.insert(
            key,
            Hardlink {
                path: item.path.clone(),
                identity,
                size: *size,
                content: content.clone(),
            },
        );
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::memory::MemoryBackend, format::tree::Entry, repo::Repository, secret::Secret,
    };
    #[test]
    fn replaced_first_copy_is_not_reused_and_next_copy_is_verified_from_chunks() {
        let scratch = tempfile::tempdir().unwrap();
        let destination = Destination::new(scratch.path()).unwrap();
        let repo = Repository::init(MemoryBackend::new(), &Secret::KeyFile([7; 32])).unwrap();
        let mut store = BlobStore::load(&repo).unwrap();
        let bytes = b"snapshot bytes";
        let id = store.add(BlobKind::Data, bytes).unwrap();
        store.finish().unwrap();
        let entry:Entry=serde_json::from_value(serde_json::json!({"name":"a","type":"file","mode":420,"mtime":"2026-11-04T14:03:22.123456789Z","ctime":"2026-11-04T14:03:22.123456789Z","uid":0,"gid":0,"inode":1,"device":1,"links":2,"size":bytes.len(),"content":[id]})).unwrap();
        let mut hardlinks = Hardlinks::new();
        let mut item = Item {
            path: "a".into(),
            entry,
            object: "tree".into(),
        };
        restore_file(&destination, &mut store, &item, &mut hardlinks).unwrap();
        std::fs::rename(scratch.path().join("a"), scratch.path().join("old-a")).unwrap();
        std::fs::write(scratch.path().join("a"), b"replacement bytes").unwrap();
        item.path = "z".into();
        restore_file(&destination, &mut store, &item, &mut hardlinks).unwrap();
        assert_eq!(std::fs::read(scratch.path().join("z")).unwrap(), bytes);
        assert_eq!(
            std::fs::read(scratch.path().join("a")).unwrap(),
            b"replacement bytes"
        );
        use std::os::unix::fs::MetadataExt;
        assert_ne!(
            std::fs::metadata(scratch.path().join("a")).unwrap().ino(),
            std::fs::metadata(scratch.path().join("z")).unwrap().ino()
        );
    }
}
