use std::io::{self, Write};

use byteorder::{BigEndian, ByteOrder};
use bytes::BytesMut;

use crate::{
    storage::{DictionaryFiles, FileLoad, FileStore, SyncableFile, TypedDictionaryFiles},
    util::sorted_iterator,
};

use super::*;

pub fn merge_string_dictionaries<
    'a,
    F: 'static + FileLoad + FileStore,
    I: Iterator<Item = &'a StringDict> + 'a,
>(
    dictionaries: I,
    dict_files: DictionaryFiles<F>,
) -> io::Result<()> {
    let iterators: Vec<_> = dictionaries.map(|d| d.iter()).collect();

    let pick_fn = |vals: &[Option<&SizedDictEntry>]| {
        vals.iter()
            .enumerate()
            .filter(|(_, v)| v.is_some())
            .min_by(|(_, x), (_, y)| x.cmp(y))
            .map(|(ix, _)| ix)
    };

    let sorted_iterator = sorted_iterator(iterators, pick_fn).map(|elt| elt.to_bytes());

    let mut blocks_file_writer = dict_files.blocks_file.open_write()?;
    let mut offsets_file_writer = dict_files.offsets_file.open_write()?;

    let mut builder = StringDictBufBuilder::new(BytesMut::new(), BytesMut::new());
    builder.add_all(sorted_iterator);
    let (offsets_buf, data_buf) = builder.finalize();

    offsets_file_writer.write_all(offsets_buf.as_ref())?;
    offsets_file_writer.flush()?;
    offsets_file_writer.sync_all()?;

    blocks_file_writer.write_all(data_buf.as_ref())?;
    blocks_file_writer.flush()?;
    blocks_file_writer.sync_all()?;

    Ok(())
}

pub fn merge_typed_dictionaries<
    'a,
    F: 'static + FileLoad + FileStore,
    I: Iterator<Item = &'a TypedDict> + 'a,
>(
    dictionaries: I,
    dict_files: TypedDictionaryFiles<F>,
) -> io::Result<()> {
    let iterators: Vec<_> = dictionaries.map(|d| d.iter()).collect();

    let pick_fn = |vals: &[Option<&TypedDictEntry>]| {
        vals.iter()
            .enumerate()
            .filter(|(_, v)| v.is_some())
            .min_by(|(_, x), (_, y)| x.cmp(y))
            .map(|(ix, _)| ix)
    };

    let sorted_iterator = sorted_iterator(iterators, pick_fn);

    let mut types_present_file_writer = dict_files.types_present_file.open_write()?;
    let mut type_offsets_file_writer = dict_files.type_offsets_file.open_write()?;
    let mut blocks_file_writer = dict_files.blocks_file.open_write()?;
    let mut offsets_file_writer = dict_files.offsets_file.open_write()?;

    let mut builder = TypedDictBufBuilder::new(
        BytesMut::new(),
        BytesMut::new(),
        BytesMut::new(),
        BytesMut::new(),
    );
    builder.add_all(sorted_iterator);
    let (types_present_buf, type_offsets_buf, offsets_buf, data_buf) = builder.finalize();

    types_present_file_writer.write_all(types_present_buf.as_ref())?;
    types_present_file_writer.flush()?;
    types_present_file_writer.sync_all()?;

    type_offsets_file_writer.write_all(type_offsets_buf.as_ref())?;
    type_offsets_file_writer.flush()?;
    type_offsets_file_writer.sync_all()?;

    offsets_file_writer.write_all(offsets_buf.as_ref())?;
    offsets_file_writer.flush()?;
    offsets_file_writer.sync_all()?;

    blocks_file_writer.write_all(data_buf.as_ref())?;
    blocks_file_writer.flush()?;
    blocks_file_writer.sync_all()?;

    Ok(())
}

pub fn dict_file_get_count<F: 'static + FileLoad>(file: &F) -> io::Result<u64> {
    let size = file.size()?;
    let mapped = file.map()?;
    let start = size - 8;
    Ok(BigEndian::read_u64(&mapped[start..]))
}
