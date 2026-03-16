use bytes::Buf;
use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive};

const TERMINAL: u8 = 0;
const FIRST_SIGN: u8 = 0b1000_0000u8;
const FIRST_TERMINAL: u8 = 0b0000_0000u8;
const CONTINUATION: u8 = 0b1000_0000u8;
const FIRST_CONTINUATION: u8 = 0b0100_0000u8;
const BASE_MASK: u8 = !CONTINUATION;
const FIRST_MASK: u8 = !(FIRST_SIGN | FIRST_CONTINUATION);
const FIRST_MAX: u8 = FIRST_CONTINUATION;
pub const NEGATIVE_ZERO: u8 = 0b0111_1111;

// Leave in reverse order for the convenience of the caller
fn size_encode(size: u32) -> Vec<u8> {
    if size == 0 {
        return vec![NEGATIVE_ZERO]; // just the positive sign bit (allows negative zero)
    }
    let mut remainder = size;
    let mut v = vec![];
    let mut last = true;
    while remainder > 0 {
        if remainder >= CONTINUATION as u32 {
            let continued = if last { TERMINAL } else { CONTINUATION };
            let byte = continued | ((remainder & BASE_MASK as u32) as u8);
            v.push(byte);
        } else if remainder >= FIRST_MAX as u32 {
            // special case where we fit in 7 bits but not 6
            // and we need a zero padded initial byte.
            let continued = if last { TERMINAL } else { CONTINUATION };
            let byte = continued | ((remainder & BASE_MASK as u32) as u8);
            v.push(byte);
            let byte = FIRST_SIGN | FIRST_CONTINUATION;
            v.push(byte)
        } else {
            let continued = if last {
                FIRST_TERMINAL
            } else {
                FIRST_CONTINUATION
            };
            let byte = FIRST_SIGN | continued | ((remainder & FIRST_MASK as u32) as u8);
            v.push(byte)
        }
        remainder >>= 7;
        last = false;
    }
    v
}

fn size_decode<B: Buf>(v: &mut B) -> (bool, u32, usize) {
    let mut size: u32 = 0;
    let mut sign = true;
    let mut i = 0;
    while v.has_remaining() {
        let vi = v.get_u8();
        if i == 0 {
            sign = vi & FIRST_SIGN != 0;
            let vi = if sign { vi } else { !vi };
            let val = (vi & FIRST_MASK) as u32;
            if vi & FIRST_CONTINUATION == 0 {
                return (sign, val, i + 1);
            } else {
                size += val
            }
        } else {
            let vi = if sign { vi } else { !vi };
            let val = (vi & BASE_MASK) as u32;
            if vi & CONTINUATION == 0 {
                return (sign, size + val, i + 1);
            } else {
                size += val
            }
        }
        size <<= 7;
        i += 1;
    }
    (sign, size, i)
}

pub fn bigint_to_storage(bigint: BigInt) -> Vec<u8> {
    let is_neg = bigint.is_negative();
    let mut int = bigint.abs();
    let size = int.bits() as u32 + 1;
    let num_bytes = (size / 8) + u32::from(size % 8 != 0);
    let size_bytes = size_encode(num_bytes);
    let mut number_vec = Vec::with_capacity(size_bytes.len() + num_bytes as usize + 1);
    for _ in 0..num_bytes {
        let byte = (&int & &BigInt::from(0xFFu8)).to_u8().unwrap_or(0);
        number_vec.push(byte);
        int >>= 8u32;
    }
    number_vec.extend(size_bytes);
    if is_neg {
        for e in number_vec.iter_mut() {
            *e = !*e;
        }
    }
    number_vec.reverse();
    number_vec
}

pub fn storage_to_bigint_and_sign<B: Buf>(bytes: &mut B) -> (BigInt, bool) {
    let (is_pos, size, _) = size_decode(bytes);
    let mut int = BigInt::from(0);
    if size == 0 {
        return (int, is_pos);
    }
    for _ in 0..size {
        int <<= 8u32;
        let b = bytes.get_u8();
        let b_val = if is_pos { b } else { !b };
        int += BigInt::from(b_val);
    }
    if !is_pos {
        int = -int;
    }
    (int, is_pos)
}

pub fn storage_to_bigint<B: Buf>(bytes: &mut B) -> BigInt {
    storage_to_bigint_and_sign(bytes).0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tfc::datatypes::{FromLexical, ToLexical};
    use crate::tfc::decimal::{decimal_to_storage, storage_to_decimal};
    use bytes::Bytes;
    use num_bigint::BigInt;
    use proptest::prelude::*;

    /// Strategy that generates arbitrary BigInt values from random byte arrays
    /// and a sign, covering a wide range of magnitudes.
    fn arb_bigint() -> impl Strategy<Value = BigInt> {
        (any::<bool>(), proptest::collection::vec(any::<u8>(), 0..64)).prop_map(
            |(positive, bytes)| {
                let sign = if positive {
                    num_bigint::Sign::Plus
                } else {
                    num_bigint::Sign::Minus
                };
                BigInt::from_bytes_be(sign, &bytes)
            },
        )
    }

    /// Strategy that generates valid decimal strings matching the format
    /// accepted by `validate_decimal`: `-?\d+(\.\d+)?`
    /// We avoid scientific notation to ensure the string round-trips exactly
    /// (scientific notation gets normalized during storage).
    fn arb_decimal_string() -> impl Strategy<Value = String> {
        (
            any::<bool>(),                                                   // negative?
            proptest::collection::vec(0..10u8, 1..20),                       // integer digits
            proptest::option::of(proptest::collection::vec(0..10u8, 1..10)), // fraction digits
        )
            .prop_map(|(negative, int_digits, frac_digits)| {
                // Build integer part, stripping leading zeros (but keep at least one digit)
                let int_str: String = int_digits.iter().map(|d| char::from(b'0' + d)).collect();
                let int_str = int_str.trim_start_matches('0');
                let int_str = if int_str.is_empty() { "0" } else { int_str };

                let sign = if negative && int_str != "0" { "-" } else { "" };

                match frac_digits {
                    Some(frac) if !frac.is_empty() => {
                        let frac_str: String = frac.iter().map(|d| char::from(b'0' + d)).collect();
                        format!("{sign}{int_str}.{frac_str}")
                    }
                    _ => format!("{sign}{int_str}"),
                }
            })
    }

    proptest! {
        /// **Validates: Requirements 4.2, 4.3, 4.4**
        ///
        /// Property 1: Typed Numeric Value Serialization Round-Trip (BigInt)
        /// For any arbitrary BigInt value, serializing it to storage bytes via
        /// `bigint_to_storage` and deserializing back via `storage_to_bigint`
        /// must produce the original value.
        #[test]
        fn prop_bigint_roundtrip(value in arb_bigint()) {
            let storage = bigint_to_storage(value.clone());
            let mut cursor = &storage[..];
            let recovered = storage_to_bigint(&mut cursor);
            prop_assert_eq!(&recovered, &value,
                "BigInt round-trip failed: original={}, recovered={}", value, recovered);
        }

        /// **Validates: Requirements 4.2, 4.3, 4.4**
        ///
        /// Property 1: Typed Numeric Value Serialization Round-Trip (Decimal)
        /// For any valid decimal string, serializing via `decimal_to_storage`
        /// and deserializing via `storage_to_decimal` must produce the original
        /// string representation.
        #[test]
        fn prop_decimal_roundtrip(value in arb_decimal_string()) {
            let storage = decimal_to_storage(&value);
            let mut cursor = &storage[..];
            let recovered = storage_to_decimal(&mut cursor);
            prop_assert_eq!(&recovered, &value,
                "Decimal round-trip failed: original='{}', recovered='{}'", value, recovered);
        }

        /// **Validates: Requirements 4.2, 4.3, 4.4**
        ///
        /// Property 1: Typed Numeric Value Serialization Round-Trip (BigInt via ToLexical/FromLexical)
        /// For any arbitrary BigInt, the higher-level `to_lexical` / `from_lexical`
        /// trait methods must also round-trip correctly.
        #[test]
        fn prop_bigint_lexical_roundtrip(value in arb_bigint()) {
            let lexical: Bytes = value.to_lexical();
            let recovered = <BigInt as FromLexical<BigInt>>::from_lexical(lexical);
            prop_assert_eq!(&recovered, &value,
                "BigInt lexical round-trip failed: original={}, recovered={}", value, recovered);
        }
    }
}
