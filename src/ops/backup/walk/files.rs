use super::*;
use crate::chunker::Chunker;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
impl<B: Backend> Walker<'_, B> {
    pub(super) fn file(&mut self, path: &Path, meta: &fs::Metadata) -> Result<Option<Node>, Error> {
        let file = match fs::OpenOptions::new()
            .read(true)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32)
            .open(path)
        {
            Ok(f) => f,
            Err(e) => {
                self.skip(path, e);
                return Ok(None);
            }
        };
        let opened = match file.metadata() {
            Ok(m) => m,
            Err(e) => {
                self.skip(path, e);
                return Ok(None);
            }
        };
        if opened.ino() != meta.ino() || opened.dev() != meta.dev() || !opened.is_file() {
            self.skip(path, "changed while opening");
            return Ok(None);
        }
        let mut reader = Chunker::new(&file, &self.gear);
        let mut content = Vec::new();
        let mut size = 0;
        loop {
            match reader.next_chunk() {
                Ok(Some(bytes)) => {
                    size += bytes.len() as u64;
                    content.push(self.store.add(crate::format::pack::BlobKind::Data, bytes)?);
                }
                Ok(None) => break,
                Err(e) => {
                    self.skip(path, e);
                    return Ok(None);
                }
            }
        }
        let after = match file.metadata() {
            Ok(m) => m,
            Err(e) => {
                self.skip(path, e);
                return Ok(None);
            }
        };
        if size != meta.len()
            || meta.mtime() != after.mtime()
            || meta.mtime_nsec() != after.mtime_nsec()
            || meta.ctime() != after.ctime()
            || meta.ctime_nsec() != after.ctime_nsec()
        {
            self.skip(path, "changed while reading");
            return Ok(None);
        }
        self.summary.files_new += 1;
        self.summary.bytes_processed += size;
        Ok(Some(Node::File { size, content }))
    }
}
