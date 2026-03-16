use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::fmt;
use std::io::{self, Write};
use std::marker::PhantomData;

pub fn find_common_prefix(b1: &[u8], b2: &[u8]) -> usize {
    let mut common = 0;
    while common < b1.len() && common < b2.len() {
        if b1[common] == b2[common] {
            common += 1;
        } else {
            break;
        }
    }

    common
}

pub fn find_common_prefix_ord(b1: &[u8], b2: &[u8]) -> (usize, Ordering) {
    let common_prefix = find_common_prefix(b1, b2);

    if common_prefix == b1.len() && b1.len() == b2.len() {
        (common_prefix, Ordering::Equal)
    } else if b1.len() == common_prefix {
        (common_prefix, Ordering::Less)
    } else if b2.len() == common_prefix {
        (common_prefix, Ordering::Greater)
    } else {
        (common_prefix, b1[common_prefix].cmp(&b2[common_prefix]))
    }
}

pub fn write_nul_terminated_bytes<W: Write>(w: &mut W, bytes: &[u8]) -> io::Result<usize> {
    w.write_all(bytes)?;
    w.write_all(&[0])?;

    let count = bytes.len() + 1;

    Ok(count)
}

/// Write padding bytes to `w`.
pub fn write_padding<W: Write>(w: &mut W, current_pos: usize, width: u8) -> io::Result<()> {
    let required_padding = (width as usize - current_pos % width as usize) % width as usize;
    w.write_all(&vec![0; required_padding])?;

    Ok(())
}

/// Write a `u64` in big-endian order to `w`.
pub fn write_u64<W: Write>(w: &mut W, num: u64) -> io::Result<()> {
    w.write_all(&num.to_be_bytes())?;

    Ok(())
}

pub struct HeapSortedIterator<'a, T: Ord, I: 'a + Iterator<Item = T> + Send> {
    iters: Vec<I>,
    heap: BinaryHeap<(Reverse<T>, usize)>,
    _x: PhantomData<&'a ()>,
}

pub fn heap_sorted_iter<'a, T: Ord, I: 'a + Iterator<Item = T> + Send>(
    mut iters: Vec<I>,
) -> HeapSortedIterator<'a, T, I> {
    let mut heap = BinaryHeap::with_capacity(iters.len());

    for (ix, i) in iters.iter_mut().enumerate() {
        if let Some(item) = i.next() {
            heap.push((Reverse(item), ix));
        }
    }

    HeapSortedIterator {
        iters,
        heap,
        _x: Default::default(),
    }
}

impl<'a, T: Ord, I: 'a + Iterator<Item = T> + Send> Iterator for HeapSortedIterator<'a, T, I> {
    type Item = T;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(ix) = self.heap.peek().map(|(_, ix)| *ix) {
            let iter = &mut self.iters[ix];
            match iter.next() {
                Some(next_item) => {
                    let item = self.heap.pop().unwrap();
                    self.heap.push((Reverse(next_item), ix));

                    Some(item.0 .0)
                }
                None => {
                    let item = self.heap.pop().unwrap();
                    Some(item.0 .0)
                }
            }
        } else {
            None
        }
    }
}

pub fn compare_or_result<T: Ord, E: fmt::Debug>(
    r1: &std::result::Result<T, E>,
    r2: &std::result::Result<T, E>,
) -> Ordering {
    if r1.is_err() {
        if r2.is_err() {
            Ordering::Equal
        } else {
            Ordering::Less
        }
    } else if r2.is_err() {
        Ordering::Greater
    } else {
        r1.as_ref().unwrap().cmp(r2.as_ref().unwrap())
    }
}

struct SortedIterator<
    T,
    I: Iterator<Item = T> + Send,
    F: 'static + Fn(&[Option<&T>]) -> Option<usize>,
> {
    iters: Vec<std::iter::Peekable<I>>,
    pick_fn: F,
}

impl<'a, T, I: 'a + Iterator<Item = T> + Send, F: 'static + Fn(&[Option<&T>]) -> Option<usize>>
    Iterator for SortedIterator<T, I, F>
{
    type Item = T;

    fn next(&mut self) -> Option<T> {
        let mut v = Vec::with_capacity(self.iters.len());
        for s in self.iters.iter_mut() {
            v.push(s.peek());
        }

        let ix = (self.pick_fn)(&v[..]);

        match ix {
            None => None,
            Some(ix) => self.iters[ix].next(),
        }
    }
}

pub fn sorted_iterator<
    'a,
    T: 'a,
    I: 'a + Iterator<Item = T> + Send,
    F: 'static + Fn(&[Option<&T>]) -> Option<usize>,
>(
    iters: Vec<I>,
    pick_fn: F,
) -> impl Iterator<Item = T> + 'a {
    let peekable_iters = iters
        .into_iter()
        .map(std::iter::Iterator::peekable)
        .collect();
    SortedIterator {
        iters: peekable_iters,
        pick_fn,
    }
}

pub fn calculate_width(size: u64) -> u8 {
    let mut msb = u64::BITS - size.leading_zeros();
    // zero is a degenerate case, but needs to be represented with one bit.
    if msb == 0 {
        msb = 1
    };
    msb as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_some_iterators() {
        let v1 = vec![1, 3, 5, 8, 12];
        let v2 = vec![7, 9, 15];
        let v3 = vec![0, 1, 2, 3, 4];

        let iters = vec![v1.into_iter(), v2.into_iter(), v3.into_iter()];

        let sorted = heap_sorted_iter(iters);

        let result: Vec<_> = sorted.collect();

        assert_eq!(vec![0, 1, 1, 2, 3, 3, 4, 5, 7, 8, 9, 12, 15], result);
    }
}
