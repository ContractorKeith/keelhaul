//! Indexes are caches: recover missing coverage from authenticated pack headers.
use super::*;
type Loaded = (
    HashMap<(BlobKind, String), Location>,
    HashMap<String, (String, Vec<Blob>)>,
    Vec<crate::damage::Damage>,
);
pub(super) fn load<B: Backend>(repo: &Repository<B>) -> Result<Loaded, Error> {
    let mut report = crate::damage::Report::default();
    let mut indexes = Vec::new();
    let mut superseded = HashSet::new();
    for entry in repo.backend.list(Kind::Index)? {
        let result = repo
            .read_object(&entry.name, Label::Index)
            .and_then(|bytes| {
                Index::parse(&bytes)
                    .map_err(|e| damage(entry.name.as_str(), "bad-format", &e.to_string()))
            });
        if let Some(index) = report.collect(result)? {
            superseded.extend(index.supersedes.iter().cloned());
            indexes.push((entry.name.as_str()[6..].to_owned(), index));
        }
    }
    let mut index = HashMap::new();
    let mut headers = HashMap::new();
    for (id, value) in indexes {
        if superseded.contains(&id) {
            continue;
        }
        for pack in value.packs {
            if let Some((_, previous)) = headers.get(&pack.id)
                && previous != &pack.blobs
            {
                report.add(
                    &format!("index/{id}"),
                    "index",
                    "index-mismatch",
                    "indexes disagree on pack header",
                );
                continue;
            }
            headers.insert(pack.id.clone(), (format!("index/{id}"), pack.blobs.clone()));
            for blob in pack.blobs {
                index
                    .entry((blob.kind, blob.id.clone()))
                    .or_insert(Location {
                        pack: pack.id.clone(),
                        blob,
                    });
            }
        }
    }
    for entry in repo.backend.list(Kind::Data)? {
        let id = entry
            .name
            .as_str()
            .rsplit('/')
            .next()
            .expect("validated pack name");
        if headers.contains_key(id) {
            continue;
        }
        if let Some(header) = report.collect(crate::repo::audit::header(
            &repo.backend,
            &repo.subkeys(),
            &entry.name,
        ))? {
            for blob in &header.blobs {
                index
                    .entry((blob.kind, blob.id.clone()))
                    .or_insert(Location {
                        pack: id.into(),
                        blob: blob.clone(),
                    });
            }
            headers.insert(id.into(), (entry.name.to_string(), header.blobs));
        }
    }
    for item in &report.damage {
        eprintln!("warning: {item}");
    }
    Ok((index, headers, report.damage))
}
