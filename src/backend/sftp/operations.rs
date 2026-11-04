use super::{
    client::{Client, payload, status},
    codec::{self, Attrs, Fields, string, u32_field},
};
use std::io;
pub struct DirectoryEntry {
    pub name: Vec<u8>,
    pub attrs: Attrs,
}
impl Client {
    pub fn open(&mut self, path: &str, flags: u32) -> io::Result<Vec<u8>> {
        let mut body = Vec::new();
        string(&mut body, path.as_bytes());
        u32_field(&mut body, flags);
        u32_field(&mut body, 0);
        let packet = self.request(3, body)?;
        let result = (|| {
            let mut fields = Fields(payload(&packet, 102)?);
            let handle = fields.string()?.to_vec();
            fields.end()?;
            Ok(handle)
        })();
        self.finish(result)
    }
    pub fn attrs(&mut self, path: &str) -> io::Result<Attrs> {
        self.attributes(17, path.as_bytes())
    }
    pub(super) fn attrs_raw(&mut self, path: &[u8]) -> io::Result<Attrs> {
        self.attributes(17, path)
    }
    pub fn lstat(&mut self, path: &str) -> io::Result<Attrs> {
        self.attributes(7, path.as_bytes())
    }
    pub(super) fn object_size(&mut self, path: &str) -> io::Result<u64> {
        let result = self.attrs(path).and_then(|attrs| attrs.regular_size());
        self.finish(result)
    }
    pub fn fstat(&mut self, handle: &[u8]) -> io::Result<Attrs> {
        self.attributes(8, handle)
    }
    fn attributes(&mut self, kind: u8, value: &[u8]) -> io::Result<Attrs> {
        let mut body = Vec::new();
        string(&mut body, value);
        let packet = self.request(kind, body)?;
        let result = (|| {
            let mut fields = Fields(payload(&packet, 105)?);
            let attrs = Attrs::decode(&mut fields)?;
            fields.end()?;
            Ok(attrs)
        })();
        self.finish(result)
    }
    pub fn mkdir(&mut self, path: &str) -> io::Result<()> {
        let mut body = Vec::new();
        string(&mut body, path.as_bytes());
        u32_field(&mut body, 0);
        let packet = self.request(14, body)?;
        let result = status(&packet);
        self.finish(result)
    }
    pub fn mkdir_all(&mut self, path: &str) -> io::Result<()> {
        let mut current = String::new();
        for part in path.split('/') {
            if part.is_empty() {
                if current.is_empty() {
                    current.push('/');
                }
                continue;
            }
            if !current.ends_with('/') && !current.is_empty() {
                current.push('/');
            }
            current.push_str(part);
            if self
                .attrs(&current)
                .is_ok_and(|a| a.permissions.is_some_and(|p| p & 0o170000 == 0o040000))
            {
                continue;
            }
            if let Err(error) = self.mkdir(&current)
                && !self
                    .attrs(&current)
                    .is_ok_and(|a| a.permissions.is_some_and(|p| p & 0o170000 == 0o040000))
            {
                return Err(error);
            }
        }
        Ok(())
    }
    pub fn remove(&mut self, path: &str) -> io::Result<()> {
        let mut body = Vec::new();
        string(&mut body, path.as_bytes());
        let packet = self.request(13, body)?;
        let result = match status(&packet) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            other => other,
        };
        self.finish(result)
    }
    pub fn rename(&mut self, from: &str, to: &str, replace: bool) -> io::Result<()> {
        let extension = replace && self.posix_rename;
        let mut body = Vec::new();
        if extension {
            string(&mut body, b"posix-rename@openssh.com");
        }
        string(&mut body, from.as_bytes());
        string(&mut body, to.as_bytes());
        let packet = self.request(if extension { 200 } else { 18 }, body)?;
        let result = status(&packet);
        self.finish(result)
    }
    pub fn directory(&mut self, path: &str) -> io::Result<Vec<DirectoryEntry>> {
        let mut body = Vec::new();
        string(&mut body, path.as_bytes());
        let packet = self.request(11, body)?;
        let result = (|| {
            let mut fields = Fields(payload(&packet, 102)?);
            let handle = fields.string()?.to_vec();
            fields.end()?;
            Ok(handle)
        })();
        let handle = self.finish(result)?;
        let result = self.directory_handle(&handle);
        let result = self.finish(result);
        let close = self.close(&handle);
        let entries = result?;
        close?;
        Ok(entries)
    }
    fn directory_handle(&mut self, handle: &[u8]) -> io::Result<Vec<DirectoryEntry>> {
        let mut entries = Vec::new();
        loop {
            let mut body = Vec::new();
            string(&mut body, handle);
            let packet = self.request(12, body)?;
            if packet.kind == 101 {
                match status(&packet) {
                    Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break,
                    Err(e) => return Err(e),
                    Ok(()) => return Err(codec::invalid("READDIR returned no entries")),
                }
            }
            let mut fields = Fields(payload(&packet, 104)?);
            let count = fields.u32()?;
            if count == 0 {
                return Err(codec::invalid("empty SFTP NAME page"));
            }
            for _ in 0..count {
                let name = fields.string()?.to_vec();
                fields.string()?;
                let attrs = Attrs::decode(&mut fields)?;
                entries.push(DirectoryEntry { name, attrs });
            }
            fields.end()?;
        }
        Ok(entries)
    }
    pub fn write_file(&mut self, path: &str, bytes: &[u8]) -> io::Result<()> {
        let handle = self.open(path, 2 | 8 | 32)?; // WRITE | CREAT | EXCL
        let result = self.write_handle(&handle, bytes);
        let close = self.close(&handle);
        result?;
        close
    }
    pub fn read_file(&mut self, path: &str, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        // Reject special files before OPEN; a FIFO could otherwise block the server.
        self.object_size(path)?;
        let handle = self.open(path, 1)?;
        self.read_open(handle, offset, len)
    }
    // Absence is a successful probe, avoiding reconnects for every new object.
    fn open_optional(&mut self, path: &str) -> io::Result<Option<Vec<u8>>> {
        let mut body = Vec::new();
        string(&mut body, path.as_bytes());
        u32_field(&mut body, 1);
        u32_field(&mut body, 0);
        let packet = self.request(3, body)?;
        if packet.kind == 101 && status(&packet).is_err_and(|e| e.kind() == io::ErrorKind::NotFound)
        {
            return Ok(None);
        }
        let result = (|| {
            let mut fields = Fields(payload(&packet, 102)?);
            Ok(fields.string()?.to_vec())
        })();
        let handle = self.finish(result)?;
        Ok(Some(handle))
    }
    pub fn matches_file(&mut self, path: &str, bytes: &[u8]) -> io::Result<Option<bool>> {
        let size = match self.object_size(path) {
            Ok(size) => size,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        if size != bytes.len() as u64 {
            return Ok(Some(false));
        }
        let Some(handle) = self.open_optional(path)? else {
            return Ok(None);
        };
        let result = (|| {
            let size = self.fstat(&handle)?.regular_size()?;
            if size != bytes.len() as u64 {
                return Ok(false);
            }
            Ok(self.read_handle(&handle, 0, size)? == bytes)
        })();
        let result = self.finish(result);
        let close = self.close(&handle);
        let matches = result?;
        close?;
        Ok(Some(matches))
    }
    fn read_open(&mut self, handle: Vec<u8>, offset: u64, len: u64) -> io::Result<Vec<u8>> {
        let result = (|| {
            let size = self.fstat(&handle)?.regular_size()?;
            self.read_handle(
                &handle,
                offset.min(size),
                len.min(size.saturating_sub(offset)),
            )
        })();
        let result = self.finish(result);
        let close = self.close(&handle);
        let bytes = result?;
        close?;
        Ok(bytes)
    }
}
