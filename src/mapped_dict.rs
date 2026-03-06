#![allow(unused)]

use super::bitindex::*;
use super::pfc::*;
use super::util::*;
use super::wavelettree::*;
use crate::storage::*;
use std::io;

pub struct MappedPfcDict {
    inner: PfcDict,
    id_wtree: Option<WaveletTree>,
}

impl MappedPfcDict {
    pub fn from_parts(dict: PfcDict, wtree: Option<WaveletTree>) -> MappedPfcDict {
        MappedPfcDict {
            inner: dict,
            id_wtree: wtree,
        }
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn get(&self, index: usize) -> Option<String> {
        if index < self.len() {
            let mapped_id = self
                .id_wtree
                .as_ref()
                .map(|wtree| wtree.lookup_one(index as u64).unwrap())
                .unwrap_or(index as u64);
            self.inner.get(mapped_id as usize)
        } else {
            None
        }
    }

    pub fn id(&self, s: &str) -> Option<u64> {
        self.inner.id(s).map(|mapped_id| {
            self.id_wtree
                .as_ref()
                .map(|wtree| wtree.decode_one(mapped_id as usize))
                .unwrap_or(mapped_id)
        })
    }
}

pub fn merge_dictionary_stack<F: 'static + FileLoad + FileStore>(
    stack: Vec<(F, Option<BitIndexFiles<F>>)>,
    dict_files: DictionaryFiles<F>,
    wavelet_files: BitIndexFiles<F>,
) -> io::Result<()> {
    // Gather counts and offsets for each dictionary in the stack
    let mut counts = Vec::with_capacity(stack.len());
    for (f, _) in stack.iter() {
        let count = dict_file_get_count(f)?;
        counts.push(count);
    }

    // Build indexed streams: for each dict, produce (index, string) pairs
    let mut all_entries: Vec<(u64, String)> = Vec::new();

    let mut offset = 0u64;
    for (i, (file, remap)) in stack.iter().enumerate() {
        let count = counts[i];
        let blocks_data = file.map()?;
        let n_strings = BigEndian::read_u64(&blocks_data[blocks_data.len() - 8..]);

        match remap {
            None => {
                // No remapping - use sequential indices starting at offset
                // We need to read strings from the dict file
                // Parse as a simple iteration over the raw PFC data
                let mut last: Option<Vec<u8>> = None;
                let mut pos = 0usize;
                let data = &blocks_data[..];

                for idx in 0..n_strings {
                    if idx % 8 == 0 {
                        // Block head: nul-terminated string
                        let end = data[pos..].iter().position(|&b| b == 0).unwrap() + pos;
                        let s = String::from_utf8(data[pos..end].to_vec()).unwrap();
                        last = Some(data[pos..end].to_vec());
                        pos = end + 1;
                        all_entries.push((offset + idx, s));
                    } else {
                        // vbyte prefix + nul-terminated suffix
                        let (prefix_len, vbyte_len) = super::vbyte::decode(&data[pos..]).unwrap();
                        pos += vbyte_len;
                        let end = data[pos..].iter().position(|&b| b == 0).unwrap() + pos;
                        let suffix = &data[pos..end];
                        let prev = last.as_ref().unwrap();
                        let mut full = Vec::with_capacity(prefix_len as usize + suffix.len());
                        full.extend_from_slice(&prev[..prefix_len as usize]);
                        full.extend_from_slice(suffix);
                        let s = String::from_utf8(full.clone()).unwrap();
                        last = Some(full);
                        pos = end + 1;
                        all_entries.push((offset + idx, s));
                    }
                }
            }
            Some(remap_files) => {
                // With remapping via wavelet tree
                let width = (count as f32).log2().ceil() as u8;
                let bi = BitIndex::from_maps(
                    remap_files.bits_file.map()?,
                    remap_files.blocks_file.map()?,
                    remap_files.sblocks_file.map()?,
                );
                let wtree = WaveletTree::from_parts(bi, width);

                // Read strings sequentially
                let mut last: Option<Vec<u8>> = None;
                let mut pos = 0usize;
                let data = &blocks_data[..];
                let mut strings = Vec::with_capacity(n_strings as usize);

                for idx in 0..n_strings {
                    if idx % 8 == 0 {
                        let end = data[pos..].iter().position(|&b| b == 0).unwrap() + pos;
                        let s = String::from_utf8(data[pos..end].to_vec()).unwrap();
                        last = Some(data[pos..end].to_vec());
                        pos = end + 1;
                        strings.push(s);
                    } else {
                        let (prefix_len, vbyte_len) = super::vbyte::decode(&data[pos..]).unwrap();
                        pos += vbyte_len;
                        let end = data[pos..].iter().position(|&b| b == 0).unwrap() + pos;
                        let suffix = &data[pos..end];
                        let prev = last.as_ref().unwrap();
                        let mut full = Vec::with_capacity(prefix_len as usize + suffix.len());
                        full.extend_from_slice(&prev[..prefix_len as usize]);
                        full.extend_from_slice(suffix);
                        let s = String::from_utf8(full.clone()).unwrap();
                        last = Some(full);
                        pos = end + 1;
                        strings.push(s);
                    }
                }

                // Use wavelet tree to remap indices
                for (decoded_idx, s) in wtree.decode().zip(strings.into_iter()) {
                    all_entries.push((decoded_idx, s));
                }
            }
        }

        offset += count;
    }

    // Sort by string value
    all_entries.sort_by(|a, b| a.1.cmp(&b.1));

    // Build the merged dictionary
    let mut builder = PfcDictFileBuilder::new(
        dict_files.blocks_file.open_write()?,
        dict_files.offsets_file.open_write()?,
    );

    let mut indexes = Vec::with_capacity(all_entries.len());
    for (ix, s) in all_entries.iter() {
        builder.add(s)?;
        indexes.push(*ix);
    }
    builder.finalize()?;

    // Build wavelet tree from the index mapping
    let max = indexes.iter().max().map(|x| *x).unwrap_or(0) + 1;
    let width = (max as f32).log2().ceil() as u8;
    build_wavelet_tree_from_iter(
        width,
        indexes.into_iter(),
        wavelet_files.bits_file,
        wavelet_files.blocks_file,
        wavelet_files.sblocks_file,
    )?;

    Ok(())
}

use byteorder::{BigEndian, ByteOrder};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::memory::*;

    #[test]
    fn mapped_dict_that_wraps_normal_dict_without_mapping() {
        let contents = vec![
            "aaaaa",
            "abcdefghijk",
            "arf",
            "bapofsi",
            "berf",
            "bzwas baraf",
            "eadfpoicvu",
            "faadsafdfaf sdfasdf",
            "gahh",
        ];

        let blocks = MemoryBackedStore::new();
        let offsets = MemoryBackedStore::new();
        let mut builder = PfcDictFileBuilder::new(
            blocks.open_write().unwrap(),
            offsets.open_write().unwrap(),
        );
        builder
            .add_all(contents.clone().into_iter())
            .unwrap();
        builder.finalize().unwrap();

        let dict = PfcDict::parse(blocks.map().unwrap(), offsets.map().unwrap()).unwrap();

        let mapped_dict = MappedPfcDict::from_parts(dict, None);

        for i in 0..contents.len() {
            let s = mapped_dict.get(i).unwrap();
            assert_eq!(contents[i], s);
            let id = mapped_dict.id(&s).unwrap();
            assert_eq!(i as u64, id);
        }
    }
}
