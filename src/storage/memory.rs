use bytes::{Bytes, BytesMut};
use std::io::{self, Cursor, Write};
use std::sync::{Arc, RwLock};

use super::types::{FileLoad, FileStore, SyncableFile};

#[derive(Clone)]
pub struct MemoryBackedStore {
    contents: Arc<RwLock<Bytes>>,
}

impl MemoryBackedStore {
    pub fn new() -> Self {
        Self {
            contents: Arc::new(RwLock::new(Bytes::new())),
        }
    }
}

pub struct MemoryBackedStoreWriter {
    buf: BytesMut,
    target: Arc<RwLock<Bytes>>,
}

impl Write for MemoryBackedStoreWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl SyncableFile for MemoryBackedStoreWriter {
    fn sync_all(&mut self) -> io::Result<()> {
        let mut contents = self.target.write().unwrap();
        *contents = self.buf.clone().freeze();
        Ok(())
    }
}

impl FileStore for MemoryBackedStore {
    type Write = MemoryBackedStoreWriter;

    fn open_write(&self) -> io::Result<Self::Write> {
        Ok(MemoryBackedStoreWriter {
            buf: BytesMut::new(),
            target: self.contents.clone(),
        })
    }
}

impl FileLoad for MemoryBackedStore {
    type Read = Cursor<Bytes>;

    fn exists(&self) -> io::Result<bool> {
        Ok(!self.contents.read().unwrap().is_empty())
    }

    fn size(&self) -> io::Result<usize> {
        Ok(self.contents.read().unwrap().len())
    }

    fn open_read_from(&self, offset: usize) -> io::Result<Cursor<Bytes>> {
        let bytes = self.contents.read().unwrap().clone();
        let mut cursor = Cursor::new(bytes);
        cursor.set_position(offset as u64);
        Ok(cursor)
    }

    fn map(&self) -> io::Result<Bytes> {
        let bytes = self.contents.read().unwrap().clone();
        if bytes.is_empty() {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "tried to open a nonexistent memory file for reading",
            ))
        } else {
            Ok(bytes)
        }
    }
}
