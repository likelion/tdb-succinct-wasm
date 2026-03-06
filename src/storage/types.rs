use bytes::{Buf, Bytes};
use std::io::{self, Read, Write};

use crate::{AdjacencyList, BitIndex};

pub trait SyncableFile: Write {
    fn sync_all(&mut self) -> io::Result<()>;
}

pub trait FileStore: Clone + Send + Sync {
    type Write: SyncableFile;
    fn open_write(&self) -> io::Result<Self::Write>;
}

pub trait FileLoad: Clone + Send + Sync {
    type Read: Read + Send;

    fn exists(&self) -> io::Result<bool>;
    fn size(&self) -> io::Result<usize>;
    fn open_read(&self) -> io::Result<Self::Read> {
        self.open_read_from(0)
    }
    fn open_read_from(&self, offset: usize) -> io::Result<Self::Read>;
    fn map(&self) -> io::Result<Bytes>;

    fn map_if_exists(&self) -> io::Result<Option<Bytes>> {
        match self.exists()? {
            false => Ok(None),
            true => {
                let mapped = self.map()?;
                Ok(Some(mapped))
            }
        }
    }
}

#[derive(Clone)]
pub struct TypedDictionaryMaps {
    pub types_present_map: Bytes,
    pub type_offsets_map: Bytes,
    pub blocks_map: Bytes,
    pub offsets_map: Bytes,
}

#[derive(Clone)]
pub struct TypedDictionaryFiles<F: 'static + FileLoad + FileStore> {
    pub types_present_file: F,
    pub type_offsets_file: F,
    pub blocks_file: F,
    pub offsets_file: F,
}

impl<F: 'static + FileLoad + FileStore> TypedDictionaryFiles<F> {
    pub fn map_all(&self) -> io::Result<TypedDictionaryMaps> {
        let types_present_map = self.types_present_file.map()?;
        let type_offsets_map = self.type_offsets_file.map()?;
        let offsets_map = self.offsets_file.map()?;
        let blocks_map = self.blocks_file.map()?;

        Ok(TypedDictionaryMaps {
            types_present_map,
            type_offsets_map,
            offsets_map,
            blocks_map,
        })
    }

    pub fn write_all_from_bufs<B1: Buf, B2: Buf, B3: Buf, B4: Buf>(
        &self,
        types_present_buf: &mut B1,
        type_offsets_buf: &mut B2,
        offsets_buf: &mut B3,
        blocks_buf: &mut B4,
    ) -> io::Result<()> {
        let mut types_present_writer = self.types_present_file.open_write()?;
        let mut type_offsets_writer = self.type_offsets_file.open_write()?;
        let mut offsets_writer = self.offsets_file.open_write()?;
        let mut blocks_writer = self.blocks_file.open_write()?;

        write_buf(&mut types_present_writer, types_present_buf)?;
        write_buf(&mut type_offsets_writer, type_offsets_buf)?;
        write_buf(&mut offsets_writer, offsets_buf)?;
        write_buf(&mut blocks_writer, blocks_buf)?;

        types_present_writer.flush()?;
        types_present_writer.sync_all()?;

        type_offsets_writer.flush()?;
        type_offsets_writer.sync_all()?;

        offsets_writer.flush()?;
        offsets_writer.sync_all()?;

        blocks_writer.flush()?;
        blocks_writer.sync_all()?;

        Ok(())
    }
}

/// Helper to write all remaining bytes from a Buf into a Write.
fn write_buf<W: Write, B: Buf>(writer: &mut W, buf: &mut B) -> io::Result<()> {
    while buf.has_remaining() {
        let chunk = buf.chunk();
        writer.write_all(chunk)?;
        let len = chunk.len();
        buf.advance(len);
    }
    Ok(())
}

#[derive(Clone)]
pub struct DictionaryMaps {
    pub blocks_map: Bytes,
    pub offsets_map: Bytes,
}

#[derive(Clone)]
pub struct DictionaryFiles<F: 'static + FileLoad + FileStore> {
    pub blocks_file: F,
    pub offsets_file: F,
}

impl<F: 'static + FileLoad + FileStore> DictionaryFiles<F> {
    pub fn map_all(&self) -> io::Result<DictionaryMaps> {
        let offsets_map = self.offsets_file.map()?;
        let blocks_map = self.blocks_file.map()?;

        Ok(DictionaryMaps {
            offsets_map,
            blocks_map,
        })
    }

    pub fn write_all_from_bufs<B1: Buf, B2: Buf>(
        &self,
        blocks_buf: &mut B1,
        offsets_buf: &mut B2,
    ) -> io::Result<()> {
        let mut offsets_writer = self.offsets_file.open_write()?;
        let mut blocks_writer = self.blocks_file.open_write()?;

        write_buf(&mut offsets_writer, offsets_buf)?;
        write_buf(&mut blocks_writer, blocks_buf)?;

        offsets_writer.flush()?;
        offsets_writer.sync_all()?;

        blocks_writer.flush()?;
        blocks_writer.sync_all()?;

        Ok(())
    }
}

#[derive(Clone)]
pub struct BitIndexMaps {
    pub bits_map: Bytes,
    pub blocks_map: Bytes,
    pub sblocks_map: Bytes,
}

impl Into<BitIndex> for BitIndexMaps {
    fn into(self) -> BitIndex {
        BitIndex::from_maps(self.bits_map, self.blocks_map, self.sblocks_map)
    }
}

#[derive(Clone)]
pub struct BitIndexFiles<F: 'static + FileLoad> {
    pub bits_file: F,
    pub blocks_file: F,
    pub sblocks_file: F,
}

impl<F: 'static + FileLoad + FileStore> BitIndexFiles<F> {
    pub fn map_all(&self) -> io::Result<BitIndexMaps> {
        let bits_map = self.bits_file.map()?;
        let blocks_map = self.blocks_file.map()?;
        let sblocks_map = self.sblocks_file.map()?;

        Ok(BitIndexMaps {
            bits_map,
            blocks_map,
            sblocks_map,
        })
    }

    pub fn map_all_if_exists(&self) -> io::Result<Option<BitIndexMaps>> {
        if self.bits_file.exists()? {
            Ok(Some(self.map_all()?))
        } else {
            Ok(None)
        }
    }
}

#[derive(Clone)]
pub struct AdjacencyListMaps {
    pub bitindex_maps: BitIndexMaps,
    pub nums_map: Bytes,
}

impl Into<AdjacencyList> for AdjacencyListMaps {
    fn into(self) -> AdjacencyList {
        AdjacencyList::parse(
            self.nums_map,
            self.bitindex_maps.bits_map,
            self.bitindex_maps.blocks_map,
            self.bitindex_maps.sblocks_map,
        )
    }
}

#[derive(Clone)]
pub struct AdjacencyListFiles<F: 'static + FileLoad> {
    pub bitindex_files: BitIndexFiles<F>,
    pub nums_file: F,
}

impl<F: 'static + FileLoad + FileStore> AdjacencyListFiles<F> {
    pub fn map_all(&self) -> io::Result<AdjacencyListMaps> {
        let bitindex_maps = self.bitindex_files.map_all()?;
        let nums_map = self.nums_file.map()?;

        Ok(AdjacencyListMaps {
            bitindex_maps,
            nums_map,
        })
    }
}

pub fn copy_file<F1: FileLoad, F2: FileStore>(f1: &F1, f2: &F2) -> io::Result<()> {
    if !f1.exists()? {
        return Ok(());
    }
    let mut input = f1.open_read()?;
    let mut output = f2.open_write()?;

    std::io::copy(&mut input, &mut output)?;
    output.flush()?;
    output.sync_all()?;

    Ok(())
}

impl<F1: 'static + FileLoad + FileStore> DictionaryFiles<F1> {
    pub fn copy_from<F2: 'static + FileLoad + FileStore>(
        &self,
        from: &DictionaryFiles<F2>,
    ) -> io::Result<()> {
        copy_file(&from.blocks_file, &self.blocks_file)?;
        copy_file(&from.offsets_file, &self.offsets_file)?;

        Ok(())
    }
}

impl<F1: 'static + FileLoad + FileStore> TypedDictionaryFiles<F1> {
    pub fn copy_from<F2: 'static + FileLoad + FileStore>(
        &self,
        from: &TypedDictionaryFiles<F2>,
    ) -> io::Result<()> {
        copy_file(&from.types_present_file, &self.types_present_file)?;
        copy_file(&from.type_offsets_file, &self.type_offsets_file)?;
        copy_file(&from.blocks_file, &self.blocks_file)?;
        copy_file(&from.offsets_file, &self.offsets_file)?;

        Ok(())
    }
}

impl<F1: 'static + FileLoad + FileStore> BitIndexFiles<F1> {
    pub fn copy_from<F2: 'static + FileLoad + FileStore>(
        &self,
        from: &BitIndexFiles<F2>,
    ) -> io::Result<()> {
        copy_file(&from.bits_file, &self.bits_file)?;
        copy_file(&from.blocks_file, &self.blocks_file)?;
        copy_file(&from.sblocks_file, &self.sblocks_file)?;

        Ok(())
    }
}

impl<F1: 'static + FileLoad + FileStore> AdjacencyListFiles<F1> {
    pub fn copy_from<F2: 'static + FileLoad + FileStore>(
        &self,
        from: &AdjacencyListFiles<F2>,
    ) -> io::Result<()> {
        copy_file(&from.nums_file, &self.nums_file)?;
        self.bitindex_files.copy_from(&from.bitindex_files)?;

        Ok(())
    }
}
