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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::io::Read;

    proptest! {
        /// **Validates: Requirements 6.3, 6.4**
        ///
        /// Property 2: MemoryBackedStore Write/Read Round-Trip
        /// For any arbitrary byte sequence, writing it to a MemoryBackedStore,
        /// calling sync_all, and then reading it back via both `map` and
        /// `open_read` must return the exact same bytes.
        #[test]
        fn prop_memory_backed_store_write_read_roundtrip(data in proptest::collection::vec(any::<u8>(), 1..1024)) {
            let store = MemoryBackedStore::new();

            // Write data and sync
            let mut writer = store.open_write().unwrap();
            writer.write_all(&data).unwrap();
            writer.sync_all().unwrap();

            // Read back via map
            let mapped = store.map().unwrap();
            prop_assert_eq!(&mapped[..], &data[..], "map() returned different data");

            // Read back via open_read
            let mut reader = store.open_read().unwrap();
            let mut read_buf = Vec::new();
            reader.read_to_end(&mut read_buf).unwrap();
            prop_assert_eq!(&read_buf, &data, "open_read() returned different data");

            // Verify size
            let size = store.size().unwrap();
            prop_assert_eq!(size, data.len(), "size() mismatch");

            // Verify exists
            prop_assert!(store.exists().unwrap(), "exists() should be true after write");
        }
    }
}
