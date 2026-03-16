use std::io::{self, Read};

use bytes::Bytes;

use num_traits::FromPrimitive;

use crate::{block::SizedDictBlock, LogArrayError, MonotonicLogArray};

use super::{
    block::{OwnedSizedBlockIterator, SizedDictReaderError},
    Datatype, SizedDictEntry, TypedDictEntry,
};

/// Synchronous iterator over a TFC dictionary's entries from a reader.
pub struct TfcDictIterator<R> {
    reader: DontReadLastU64Reader<R>,
    current_block: Option<OwnedSizedBlockIterator>,
    start_of_block: bool,
    done: bool,
}

impl<R: Read> TfcDictIterator<R> {
    pub fn new(reader: R) -> Self {
        Self {
            reader: DontReadLastU64Reader::new(reader),
            current_block: None,
            start_of_block: false,
            done: false,
        }
    }
}

impl<R: Read> Iterator for TfcDictIterator<R> {
    type Item = Result<(SizedDictEntry, bool), SizedDictReaderError>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        loop {
            if let Some(ref mut iter) = self.current_block {
                if let Some(entry) = iter.next() {
                    let sob = self.start_of_block;
                    self.start_of_block = false;
                    return Some(Ok((entry, sob)));
                }
            }

            // Try to read the next block
            match SizedDictBlock::parse_from_reader(&mut self.reader) {
                Ok(block) => {
                    self.start_of_block = true;
                    self.current_block = Some(block.into_iter());
                    continue;
                }
                Err(e) => {
                    if e.is_unexpected_eof() {
                        self.done = true;
                        return None;
                    }
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

/// Synchronous iterator over a typed TFC dictionary's entries from a reader.
pub struct TfcTypedDictIterator<R> {
    inner: TfcDictIterator<R>,
    types_present: MonotonicLogArray,
    type_offsets: MonotonicLogArray,
    block_index: usize,
    offset: usize,
}

impl<R: Read> TfcTypedDictIterator<R> {
    pub fn from_parts(
        blocks_reader: R,
        types_present: MonotonicLogArray,
        type_offsets: MonotonicLogArray,
    ) -> Self {
        Self {
            inner: TfcDictIterator::new(blocks_reader),
            types_present,
            type_offsets,
            block_index: 0,
            offset: 0,
        }
    }

    pub fn new(
        blocks_reader: R,
        types_present_bytes: Bytes,
        type_offsets_bytes: Bytes,
    ) -> Result<Self, LogArrayError> {
        let types_present = MonotonicLogArray::parse(types_present_bytes)?;
        let type_offsets = MonotonicLogArray::parse(type_offsets_bytes)?;

        Ok(Self::from_parts(blocks_reader, types_present, type_offsets))
    }
}

impl<R: Read> Iterator for TfcTypedDictIterator<R> {
    type Item = Result<TypedDictEntry, SizedDictReaderError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.inner.next() {
            Some(Ok((d, b))) => {
                if b {
                    if self.block_index != 0
                        && self.offset < self.type_offsets.len()
                        && self.block_index as u64 == self.type_offsets.entry(self.offset) + 1
                    {
                        self.offset += 1;
                    }
                    self.block_index += 1;
                }
                let data_type = Datatype::from_u64(self.types_present.entry(self.offset)).unwrap();
                Some(Ok(TypedDictEntry::new(data_type, d)))
            }
            Some(Err(e)) => Some(Err(e)),
            None => None,
        }
    }
}

/// A reader wrapper that withholds the last 8 bytes (the control word).
struct DontReadLastU64Reader<R> {
    inner: R,
    buf: [u8; 8],
    buf_len: usize,
    initialized: bool,
}

impl<R> DontReadLastU64Reader<R> {
    pub fn new(r: R) -> Self {
        Self {
            inner: r,
            buf: [0; 8],
            buf_len: 0,
            initialized: false,
        }
    }
}

impl<R: Read> Read for DontReadLastU64Reader<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if !self.initialized {
            // Read the first 8 bytes into our buffer
            match self.inner.read_exact(&mut self.buf) {
                Ok(()) => {
                    self.buf_len = 8;
                    self.initialized = true;
                }
                Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    self.initialized = true;
                    return Err(e);
                }
                Err(e) => return Err(e),
            }
        }

        if self.buf_len == 0 {
            return Ok(0);
        }

        // Read new data from inner
        let mut temp = vec![0u8; out.len()];
        let n = match self.inner.read(&mut temp) {
            Ok(n) => n,
            Err(e) => return Err(e),
        };

        if n == 0 {
            // Inner is exhausted. The buf contains the last 8 bytes (control word).
            // Don't return them.
            self.buf_len = 0;
            return Ok(0);
        }

        // We have new data. Output from our buffer and shift new data in.
        let to_output = std::cmp::min(n, out.len());
        // Copy from buf to output
        let from_buf = std::cmp::min(to_output, self.buf_len);
        out[..from_buf].copy_from_slice(&self.buf[..from_buf]);

        if to_output > from_buf {
            // Also copy some from temp
            let from_temp = to_output - from_buf;
            out[from_buf..to_output].copy_from_slice(&temp[..from_temp]);
        }

        // Now update our buffer: shift remaining buf bytes + new temp bytes
        let mut new_buf_data = Vec::with_capacity(self.buf_len + n);
        if from_buf < self.buf_len {
            new_buf_data.extend_from_slice(&self.buf[from_buf..self.buf_len]);
        }
        new_buf_data.extend_from_slice(&temp[..n]);

        // Remove the first `to_output - from_buf` bytes from temp that we already output
        let already_output_from_temp = if to_output > from_buf {
            to_output - from_buf
        } else {
            0
        };
        let remaining = &new_buf_data[already_output_from_temp..];

        self.buf_len = std::cmp::min(remaining.len(), 8);
        self.buf[..self.buf_len].copy_from_slice(&remaining[remaining.len() - self.buf_len..]);

        Ok(to_output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    use bytes::{Bytes, BytesMut};

    use crate::{StringDictBufBuilder, TdbDataType, TypedDictBufBuilder};

    #[test]
    fn stream_a_dict() {
        let mut builder = StringDictBufBuilder::new(BytesMut::new(), BytesMut::new());
        let input = vec![
            Bytes::from("aaa".to_string()),
            Bytes::from("aab".to_string()),
            Bytes::from("aac".to_string()),
            Bytes::from("aad".to_string()),
            Bytes::from("aba".to_string()),
            Bytes::from("abb".to_string()),
            Bytes::from("abc".to_string()),
            Bytes::from("abd".to_string()),
            Bytes::from("baa".to_string()),
            Bytes::from("bab".to_string()),
        ];
        builder.add_all(input.iter().cloned());

        let (_, data) = builder.finalize();

        let iter = TfcDictIterator::new(data.as_ref());
        let result: Vec<_> = iter.collect::<Result<Vec<_>, _>>().unwrap();
        let boundary_result: Vec<bool> = result.iter().map(|(_, b)| *b).collect();
        let data_result: Vec<Bytes> = result.into_iter().map(|(e, _)| e.to_bytes()).collect();
        assert_eq!(input, data_result);
        assert_eq!(
            vec![true, false, false, false, false, false, false, false, true, false],
            boundary_result
        );
    }

    fn typed_dict_test(mut input: Vec<TypedDictEntry>) {
        input.sort();

        let mut builder = TypedDictBufBuilder::new(
            BytesMut::new(),
            BytesMut::new(),
            BytesMut::new(),
            BytesMut::new(),
        );

        builder.add_all(input.iter().cloned());
        let (types_present, type_offsets, _, data) = builder.finalize();
        let iter =
            TfcTypedDictIterator::new(data.as_ref(), types_present.freeze(), type_offsets.freeze())
                .unwrap();
        let result: Vec<_> = iter.collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(input, result);
    }

    #[test]
    fn test_a_typed_dict() {
        let input = vec![
            String::make_entry(&"a fun string"),
            String::make_entry(&"a fun string2"),
            String::make_entry(&"a fun string3"),
            String::make_entry(&"a fun string4"),
            String::make_entry(&"a fun string5"),
            String::make_entry(&"a fun string6"),
            String::make_entry(&"a fun string7"),
            String::make_entry(&"a fun string8"),
            String::make_entry(&"a fun string9"),
            u32::make_entry(&25),
            u32::make_entry(&42),
            u32::make_entry(&65),
            u32::make_entry(&66),
            u32::make_entry(&67),
            u32::make_entry(&68),
            u32::make_entry(&69),
            u32::make_entry(&75),
            u32::make_entry(&85),
            f64::make_entry(&3.1415),
        ];

        typed_dict_test(input);
    }

    #[test]
    fn single_element_typed_dict() {
        let input = vec![String::make_entry(&"a fun string")];
        typed_dict_test(input);
    }

    #[test]
    fn empty_typed_dict() {
        let input = vec![];
        typed_dict_test(input);
    }

    #[test]
    fn read_small_buf() {
        let data = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let mut reader = DontReadLastU64Reader::new(data.as_ref());
        let mut buf = [0u8; 1];
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(0, buf[0]);
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(1, buf[0]);
        reader.read_exact(&mut buf).unwrap();
        assert_eq!(2, buf[0]);
        // The last 8 bytes are withheld, so reading more should fail
        assert!(
            reader.read_exact(&mut buf).is_err() || {
                let mut remaining = Vec::new();
                reader.read_to_end(&mut remaining).unwrap();
                remaining.is_empty()
            }
        );
    }

    #[test]
    fn read_large_buf() {
        let data = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let mut reader = DontReadLastU64Reader::new(data.as_ref());
        let mut output = Vec::new();
        reader.read_to_end(&mut output).unwrap();
        assert_eq!(vec![0, 1, 2], output);
    }
}
