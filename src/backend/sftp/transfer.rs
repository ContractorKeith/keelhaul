use super::{
    client::{Client, WINDOW, payload, status},
    codec::{Fields, string, u32_field, u64_field},
};
use std::{collections::BTreeMap, io};
const CHUNK: usize = 32 * 1024;
impl Client {
    pub fn write_handle(&mut self, handle: &[u8], bytes: &[u8]) -> io::Result<()> {
        let result = self.write_pending(handle, bytes);
        self.finish(result)
    }
    pub fn read_handle(&mut self, handle: &[u8], offset: u64, len: u64) -> io::Result<Vec<u8>> {
        let result = self.read_pending(handle, offset, len);
        self.finish(result)
    }
    fn write_pending(&mut self, handle: &[u8], bytes: &[u8]) -> io::Result<()> {
        for (batch, chunks) in bytes.chunks(CHUNK * WINDOW).enumerate() {
            let mut pending = BTreeMap::new();
            for (index, chunk) in chunks.chunks(CHUNK).enumerate() {
                let mut body = Vec::new();
                string(&mut body, handle);
                u64_field(&mut body, (batch * CHUNK * WINDOW + index * CHUNK) as u64);
                string(&mut body, chunk);
                pending.insert(self.send(6, body)?, ());
            }
            let mut failure = None;
            while !pending.is_empty() {
                let (id, packet) = self.receive()?;
                if pending.remove(&id).is_none() {
                    return Err(self.protocol_error("unexpected WRITE request ID"));
                }
                if let Err(error) = status(&packet) {
                    failure.get_or_insert(error);
                }
            }
            if let Some(error) = failure {
                return Err(error);
            }
        }
        Ok(())
    }
    fn read_pending(
        &mut self,
        handle: &[u8],
        mut offset: u64,
        mut len: u64,
    ) -> io::Result<Vec<u8>> {
        if offset.checked_add(len).is_none() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SFTP read range overflows",
            ));
        }
        let mut output = Vec::new();
        while len > 0 {
            let mut pending = BTreeMap::new();
            for index in 0..WINDOW {
                let start = index as u64 * CHUNK as u64;
                if start >= len {
                    break;
                }
                let count = (len - start).min(CHUNK as u64) as u32;
                let mut body = Vec::new();
                string(&mut body, handle);
                u64_field(&mut body, offset + start);
                u32_field(&mut body, count);
                pending.insert(self.send(5, body)?, (index, count));
            }
            let mut chunks = BTreeMap::new();
            let mut failure = None;
            while !pending.is_empty() {
                let (id, packet) = self.receive()?;
                let Some((index, count)) = pending.remove(&id) else {
                    return Err(self.protocol_error("unexpected READ request ID"));
                };
                let body = match payload(&packet, 103) {
                    Ok(body) => body,
                    Err(error) => {
                        failure.get_or_insert(error);
                        continue;
                    }
                };
                let mut fields = Fields(body);
                let data = fields.string()?.to_vec();
                fields.end()?;
                if data.is_empty() || data.len() > count as usize {
                    return Err(self.protocol_error("invalid READ data length"));
                }
                chunks.insert(index, (data, count));
            }
            if let Some(error) = failure {
                return Err(error);
            }
            // A server may legally return short DATA. Reissue its missing suffix.
            for (_, (mut data, count)) in chunks {
                if data.len() < count as usize {
                    while data.len() < count as usize {
                        let mut body = Vec::new();
                        string(&mut body, handle);
                        u64_field(&mut body, offset + data.len() as u64);
                        u32_field(&mut body, count - data.len() as u32);
                        let packet = self.request(5, body)?;
                        let mut fields = Fields(payload(&packet, 103)?);
                        let suffix = fields.string()?;
                        if suffix.is_empty() || suffix.len() > count as usize - data.len() {
                            return Err(self.protocol_error("invalid short READ data"));
                        }
                        data.extend_from_slice(suffix);
                        fields.end()?;
                    }
                }
                offset += data.len() as u64;
                len -= data.len() as u64;
                output.extend(data);
            }
        }
        Ok(output)
    }
}
