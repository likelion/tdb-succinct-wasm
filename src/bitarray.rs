#![allow(clippy::precedence, clippy::verbose_bit_mask)]

//! Code for reading, writing, and using bit arrays.

use super::util;
use crate::storage::{FileLoad, SyncableFile};
use byteorder::{BigEndian, ByteOrder};
use bytes::{Buf, BufMut, Bytes};
use std::io;
use std::{convert::TryFrom, error, fmt};

/// A thread-safe, reference-counted, compressed bit sequence.
#[derive(Clone)]
pub struct BitArray {
    len: u64,
    buf: Bytes,
}

#[derive(Debug, PartialEq)]
pub enum BitArrayError {
    InputBufferTooSmall(usize),
    UnexpectedInputBufferSize(u64, u64, u64),
}

impl BitArrayError {
    fn validate_input_buf_size(input_buf_size: usize) -> Result<(), Self> {
        if input_buf_size < 8 {
            return Err(BitArrayError::InputBufferTooSmall(input_buf_size));
        }
        Ok(())
    }

    fn validate_len(input_buf_size: usize, len: u64) -> Result<(), Self> {
        let expected_buf_size = {
            let after_shifting = len >> 6 << 3;
            if len & 63 == 0 {
                after_shifting + 8
            } else {
                after_shifting + 16
            }
        };
        let input_buf_size = u64::try_from(input_buf_size).unwrap();

        if input_buf_size != expected_buf_size {
            return Err(BitArrayError::UnexpectedInputBufferSize(
                input_buf_size,
                expected_buf_size,
                len,
            ));
        }

        Ok(())
    }
}

impl fmt::Display for BitArrayError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use BitArrayError::*;
        match self {
            InputBufferTooSmall(input_buf_size) => {
                write!(f, "expected input buffer size ({}) >= 8", input_buf_size)
            }
            UnexpectedInputBufferSize(input_buf_size, expected_buf_size, len) => write!(
                f,
                "expected input buffer size ({}) to be {} for {} bits",
                input_buf_size, expected_buf_size, len
            ),
        }
    }
}

impl error::Error for BitArrayError {}

impl From<BitArrayError> for io::Error {
    fn from(err: BitArrayError) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, err)
    }
}

fn read_control_word(buf: &[u8], input_buf_size: usize) -> Result<u64, BitArrayError> {
    let len = BigEndian::read_u64(buf);
    BitArrayError::validate_len(input_buf_size, len)?;
    Ok(len)
}

impl BitArray {
    pub fn from_bits(mut buf: Bytes) -> Result<BitArray, BitArrayError> {
        let input_buf_size = buf.len();
        BitArrayError::validate_input_buf_size(input_buf_size)?;

        let len = read_control_word(&buf.split_off(input_buf_size - 8), input_buf_size)?;

        Ok(BitArray { buf, len })
    }

    pub fn bits(&self) -> &[u8] {
        &self.buf
    }

    pub fn len(&self) -> usize {
        usize::try_from(self.len).unwrap_or_else(|_| {
            panic!(
                "expected length ({}) to fit in {} bytes",
                self.len,
                std::mem::size_of::<usize>()
            )
        })
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn get(&self, index: usize) -> bool {
        let len = self.len();
        debug_assert!(index < len, "expected index ({}) < length ({})", index, len);

        let byte = self.buf[index / 8];
        let mask = 0b1000_0000 >> index % 8;

        byte & mask != 0
    }

    pub fn iter(&self) -> impl Iterator<Item = bool> {
        let bits = self.clone();
        (0..bits.len()).map(move |index| bits.get(index))
    }
}

pub struct BitArrayBufBuilder<B> {
    dest: B,
    current: u64,
    count: u64,
}

impl<B: BufMut> BitArrayBufBuilder<B> {
    pub fn new(dest: B) -> BitArrayBufBuilder<B> {
        BitArrayBufBuilder {
            dest,
            current: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, bit: bool) {
        if bit {
            let pos = self.count & 0b11_1111;
            self.current |= 0x8000_0000_0000_0000 >> pos;
        }
        self.count += 1;
        if self.count & 0b11_1111 == 0 {
            self.dest.put_u64(self.current);
            self.current = 0;
        }
    }

    pub fn push_all<I: Iterator<Item = bool>>(&mut self, mut iter: I) {
        while let Some(bit) = iter.next() {
            self.push(bit);
        }
    }

    fn finalize_data(&mut self) {
        if self.count & 0b11_1111 != 0 {
            self.dest.put_u64(self.current);
        }
    }

    pub fn finalize(mut self) -> B {
        let count = self.count;
        self.finalize_data();
        self.dest.put_u64(count);
        self.dest
    }

    pub fn count(&self) -> u64 {
        self.count
    }
}

pub struct BitArrayFileBuilder<W> {
    dest: W,
    current: u64,
    count: u64,
}

impl<W: SyncableFile> BitArrayFileBuilder<W> {
    pub fn new(dest: W) -> BitArrayFileBuilder<W> {
        BitArrayFileBuilder {
            dest,
            current: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, bit: bool) -> io::Result<()> {
        if bit {
            let pos = self.count & 0b11_1111;
            self.current |= 0x8000_0000_0000_0000 >> pos;
        }
        self.count += 1;
        if self.count & 0b11_1111 == 0 {
            util::write_u64(&mut self.dest, self.current)?;
            self.current = 0;
        }
        Ok(())
    }

    pub fn push_all<I: Iterator<Item = bool>>(&mut self, iter: I) -> io::Result<()> {
        for bit in iter {
            self.push(bit)?;
        }
        Ok(())
    }

    fn finalize_data(&mut self) -> io::Result<()> {
        if self.count & 0b11_1111 != 0 {
            util::write_u64(&mut self.dest, self.current)?;
        }
        Ok(())
    }

    pub fn finalize(mut self) -> io::Result<()> {
        let count = self.count;
        self.finalize_data()?;
        util::write_u64(&mut self.dest, count)?;
        self.dest.flush()?;
        self.dest.sync_all()?;
        Ok(())
    }

    pub fn count(&self) -> u64 {
        self.count
    }
}

fn decode_next_bitarray_block<B: Buf>(bytes: &mut B, readahead: &mut Option<u64>) -> Option<u64> {
    if bytes.remaining() < 8 {
        return None;
    }
    match readahead.replace(bytes.get_u64()) {
        Some(word) => Some(word),
        None => decode_next_bitarray_block(bytes, readahead),
    }
}

pub fn bitarray_iter_blocks<B: Buf>(b: B) -> BitArrayBlockIterator<B> {
    BitArrayBlockIterator {
        buf: b,
        readahead: None,
    }
}

pub struct BitArrayBlockIterator<B: Buf> {
    buf: B,
    readahead: Option<u64>,
}

impl<B: Buf> Iterator for BitArrayBlockIterator<B> {
    type Item = u64;
    fn next(&mut self) -> Option<u64> {
        decode_next_bitarray_block(&mut self.buf, &mut self.readahead)
    }
}

/// Read the length (number of bits) from a `FileLoad`.
pub fn bitarray_len_from_file<F: FileLoad>(f: &F) -> io::Result<u64> {
    let size = f.size()?;
    BitArrayError::validate_input_buf_size(size)?;
    let mapped = f.map()?;
    Ok(read_control_word(&mapped[size - 8..], size)?)
}

#[cfg(test)]
mod tests {
    use crate::storage::memory::MemoryBackedStore;
    use crate::storage::FileStore;

    use super::*;

    #[test]
    fn bit_array_error() {
        assert_eq!(
            "expected input buffer size (7) >= 8",
            BitArrayError::InputBufferTooSmall(7).to_string()
        );
        assert_eq!(
            "expected input buffer size (9) to be 8 for 0 bits",
            BitArrayError::UnexpectedInputBufferSize(9, 8, 0).to_string()
        );
        assert_eq!(
            io::Error::new(
                io::ErrorKind::InvalidData,
                BitArrayError::InputBufferTooSmall(7)
            )
            .to_string(),
            io::Error::from(BitArrayError::InputBufferTooSmall(7)).to_string()
        );
    }

    #[test]
    fn validate_input_buf_size() {
        let val = |buf_size| BitArrayError::validate_input_buf_size(buf_size);
        let err = |buf_size| Err(BitArrayError::InputBufferTooSmall(buf_size));
        assert_eq!(err(7), val(7));
        assert_eq!(Ok(()), val(8));
        assert_eq!(Ok(()), val(9));
        assert_eq!(Ok(()), val(usize::max_value()));
    }

    #[test]
    fn validate_len() {
        let val = |buf_size, len| BitArrayError::validate_len(buf_size, len);
        let err = |buf_size, expected, len| {
            Err(BitArrayError::UnexpectedInputBufferSize(
                buf_size, expected, len,
            ))
        };

        assert_eq!(err(0, 8, 0), val(0, 0));
        assert_eq!(Ok(()), val(16, 1));
        assert_eq!(Ok(()), val(16, 2));

        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            Ok(()),
            val(
                usize::try_from(u128::from(u64::max_value()) + 65 >> 6 << 3).unwrap(),
                u64::max_value()
            )
        );
    }

    #[test]
    fn empty() {
        assert!(BitArray::from_bits(Bytes::from([0u8; 8].as_ref()))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn construct_and_parse_small_bitarray() {
        let x = MemoryBackedStore::new();
        let contents = vec![true, true, false, false, true];

        let mut builder = BitArrayFileBuilder::new(x.open_write().unwrap());
        builder.push_all(contents.into_iter()).unwrap();
        builder.finalize().unwrap();

        let loaded = x.map().unwrap();

        let bitarray = BitArray::from_bits(loaded).unwrap();

        assert_eq!(true, bitarray.get(0));
        assert_eq!(true, bitarray.get(1));
        assert_eq!(false, bitarray.get(2));
        assert_eq!(false, bitarray.get(3));
        assert_eq!(true, bitarray.get(4));
    }

    #[test]
    fn construct_and_parse_large_bitarray() {
        let x = MemoryBackedStore::new();
        let contents = (0..).map(|n| n % 3 == 0).take(123456);

        let mut builder = BitArrayFileBuilder::new(x.open_write().unwrap());
        builder.push_all(contents).unwrap();
        builder.finalize().unwrap();

        let loaded = x.map().unwrap();

        let bitarray = BitArray::from_bits(loaded).unwrap();

        for i in 0..bitarray.len() {
            assert_eq!(i % 3 == 0, bitarray.get(i));
        }
    }

    #[test]
    fn bitarray_len_from_file_errors() {
        use std::io::Write;
        let store = MemoryBackedStore::new();
        let mut writer = store.open_write().unwrap();
        writer.write_all(&[0, 0, 0]).unwrap();
        writer.sync_all().unwrap();
        assert_eq!(
            io::Error::from(BitArrayError::InputBufferTooSmall(3)).to_string(),
            bitarray_len_from_file(&store).err().unwrap().to_string()
        );

        let store = MemoryBackedStore::new();
        let mut writer = store.open_write().unwrap();
        writer.write_all(&[0, 0, 0, 0, 0, 0, 0, 2]).unwrap();
        writer.sync_all().unwrap();
        assert_eq!(
            io::Error::from(BitArrayError::UnexpectedInputBufferSize(8, 16, 2)).to_string(),
            bitarray_len_from_file(&store).err().unwrap().to_string()
        );
    }
}
