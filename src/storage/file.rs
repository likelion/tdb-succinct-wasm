use std::io::{self, BufWriter, Read, Seek, SeekFrom};
use std::path::PathBuf;

use bytes::{Bytes, BytesMut};

use super::{FileLoad, FileStore, SyncableFile};

#[derive(Clone, Debug)]
pub struct FileBackedStore {
    path: PathBuf,
}

impl SyncableFile for std::fs::File {
    fn sync_all(&mut self) -> io::Result<()> {
        std::fs::File::sync_all(self)
    }
}

impl SyncableFile for BufWriter<std::fs::File> {
    fn sync_all(&mut self) -> io::Result<()> {
        let inner = self.get_mut();
        std::fs::File::sync_all(inner)
    }
}

impl FileBackedStore {
    pub fn new<P: Into<PathBuf>>(path: P) -> FileBackedStore {
        FileBackedStore { path: path.into() }
    }
}

impl FileLoad for FileBackedStore {
    type Read = std::fs::File;

    fn exists(&self) -> io::Result<bool> {
        let metadata = std::fs::metadata(&self.path);
        Ok(!(metadata.is_err() && metadata.err().unwrap().kind() == io::ErrorKind::NotFound))
    }

    fn size(&self) -> io::Result<usize> {
        let m = std::fs::metadata(&self.path)?;
        Ok(m.len() as usize)
    }

    fn open_read_from(&self, offset: usize) -> io::Result<std::fs::File> {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .open(&self.path)?;

        file.seek(SeekFrom::Start(offset as u64))?;

        Ok(file)
    }

    fn map(&self) -> io::Result<Bytes> {
        let size = self.size()?;
        if size == 0 {
            Ok(Bytes::new())
        } else {
            let mut f = self.open_read()?;
            let mut b = BytesMut::with_capacity(size);

            // unsafe justification: We are immediately
            // overwriting the data in this BytesMut with the file
            // contents, so it doesn't matter that it is
            // uninitialized.
            // Should file reading fail, an error will be
            // returned, and the BytesMut will be freed, ensuring
            // nobody ever looks at the uninitialized data.
            unsafe { b.set_len(size) };
            f.read_exact(&mut b[..])?;
            Ok(b.freeze())
        }
    }
}

impl FileStore for FileBackedStore {
    type Write = BufWriter<std::fs::File>;

    fn open_write(&self) -> io::Result<BufWriter<std::fs::File>> {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .open(&self.path)?;

        Ok(BufWriter::new(file))
    }
}
