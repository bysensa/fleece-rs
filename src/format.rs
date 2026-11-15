//! Bit layout of Fleece values.
//!
//! ```text
//! 0000iiii iiiiiiii       small integer (12-bit, signed)
//! 0001uccc iiiiiiii...    long integer (u = unsigned, ccc = byte count - 1), LE
//! 0010sx-- --------...    floating point (s = 64-bit, x = a double stored in 32 bits), LE
//! 0011ss-- --------       special (0 null, 1 false, 2 true, 3 undefined)
//! 00110001 kkkkkkkk...    extension of this crate: a value of kind k, its payload follows
//! 0100cccc ssssssss...    string (cccc = byte count, 15 = a varint count follows)
//! 0101cccc dddddddd...    binary data (same as string)
//! 0110wccc cccccccc...    array (11-bit count, 2047 = a varint with the rest follows; w = wide)
//! 0111wccc cccccccc...    dictionary (same as array, the count is of key/value pairs)
//! 1xoooooo oooooooo       pointer (x = into the base document; o = BE offset back, in 2 bytes)
//! ```

pub(crate) const TAG_SHORT_INT: u8 = 0;
pub(crate) const TAG_INT: u8 = 1;
pub(crate) const TAG_FLOAT: u8 = 2;
pub(crate) const TAG_SPECIAL: u8 = 3;
pub(crate) const TAG_STRING: u8 = 4;
pub(crate) const TAG_DATA: u8 = 5;
pub(crate) const TAG_ARRAY: u8 = 6;
pub(crate) const TAG_DICT: u8 = 7;

pub(crate) const SPECIAL_NULL: u8 = 0x30;
pub(crate) const SPECIAL_EXT: u8 = 0x31;
pub(crate) const SPECIAL_FALSE: u8 = 0x34;
pub(crate) const SPECIAL_TRUE: u8 = 0x38;
pub(crate) const SPECIAL_UNDEFINED: u8 = 0x3C;

pub(crate) const FLOAT_32: u8 = 0x20;
pub(crate) const FLOAT_32_DOUBLE: u8 = 0x24;
pub(crate) const FLOAT_64: u8 = 0x28;

/// The count in a collection's header that says a varint with the rest follows.
pub(crate) const LONG_COUNT: usize = 0x07FF;
/// The key of the entry that points at the dictionary this one inherits from.
pub(crate) const PARENT_KEY: i64 = -2048;
/// The farthest a pointer of a narrow collection reaches back, in bytes.
pub(crate) const MAX_NARROW_OFFSET: usize = 0x3FFF * 2;
/// The farthest a pointer of a wide collection reaches back, in bytes.
pub(crate) const MAX_WIDE_OFFSET: usize = 0x3FFF_FFFF * 2;

/// The version of the bytes this crate writes: the layout of standard Fleece and the kinds of
/// [`ext`] with their payloads. Documents carry no version of their own; an application keeps
/// this number once beside the documents it stores, and so knows what wrote them. It goes up
/// when a value is written as other bytes than before, and never when a kind is only added.
pub const FORMAT_VERSION: u32 = 1;

/// How deep collections may nest, in a document read and in a value written.
pub const MAX_DEPTH: usize = 128;

/// The kinds of extended values: what follows the byte `0x31`.
pub mod ext {
    /// No value at all (SurrealDB's `NONE`). The only kind without a payload.
    pub const NONE: u8 = 0;
    /// `i8`, payload int.
    pub const I8: u8 = 1;
    /// `u8`, payload int.
    pub const U8: u8 = 2;
    /// `i16`, payload int.
    pub const I16: u8 = 3;
    /// `u16`, payload int.
    pub const U16: u8 = 4;
    /// `i32`, payload int.
    pub const I32: u8 = 5;
    /// `u32`, payload int.
    pub const U32: u8 = 6;
    /// `u64`, payload int.
    pub const U64: u8 = 7;
    /// `f16`, payload int holding the 16 bits.
    pub const F16: u8 = 8;
    /// `decimal`, payload data of 16 bytes.
    pub const DECIMAL: u8 = 9;
    /// `bigdecimal`, payload array `[scale, coefficient]`, the coefficient as data.
    pub const BIG_DECIMAL: u8 = 10;
    /// `duration`, payload data of 12 bytes.
    pub const DURATION: u8 = 11;
    /// `datetime`, payload data of 12 bytes.
    pub const DATETIME: u8 = 12;
    /// `uuid`, payload data of 16 bytes.
    pub const UUID: u8 = 13;
    /// `date`, payload int.
    pub const DATE: u8 = 14;
    /// `time`, payload int.
    pub const TIME: u8 = 15;
    /// `timestamp`, payload data of 12 bytes.
    pub const TIMESTAMP: u8 = 16;
    /// `interval`, payload data of 16 bytes.
    pub const INTERVAL: u8 = 17;
    /// `table`, payload string.
    pub const TABLE: u8 = 18;
    /// `record`, payload array `[table, key]`.
    pub const RECORD_ID: u8 = 19;
    /// `file`, payload array `[bucket, key]`.
    pub const FILE: u8 = 20;
    /// `range`, payload array `[bound flags, start, end]`.
    pub const RANGE: u8 = 21;
    /// `regex`, payload string.
    pub const REGEX: u8 = 22;
    /// `set`, payload array.
    pub const SET: u8 = 23;
    /// `geometry`, payload array `[geometry kind, coordinates]`.
    pub const GEOMETRY: u8 = 24;
    /// The greatest kind this version knows.
    pub const MAX: u8 = GEOMETRY;

    /// The name of a kind, as SurrealDB spells the type.
    pub fn name(kind: u8) -> Option<&'static str> {
        Some(match kind {
            NONE => "none",
            I8 => "i8",
            U8 => "u8",
            I16 => "i16",
            U16 => "u16",
            I32 => "i32",
            U32 => "u32",
            U64 => "u64",
            F16 => "f16",
            DECIMAL => "decimal",
            BIG_DECIMAL => "bigdecimal",
            DURATION => "duration",
            DATETIME => "datetime",
            UUID => "uuid",
            DATE => "date",
            TIME => "time",
            TIMESTAMP => "timestamp",
            INTERVAL => "interval",
            TABLE => "table",
            RECORD_ID => "record",
            FILE => "file",
            RANGE => "range",
            REGEX => "regex",
            SET => "set",
            GEOMETRY => "geometry",
            _ => return None,
        })
    }
}

/// `n` rounded up to an even number.
pub(crate) fn even(n: usize) -> usize {
    n + (n & 1)
}

pub(crate) fn put_varint(out: &mut Vec<u8>, mut n: u64) {
    while n >= 0x80 {
        out.push(n as u8 | 0x80);
        n >>= 7;
    }
    out.push(n as u8);
}

/// The varint at the start of `buf` and the number of bytes it takes.
pub(crate) fn get_varint(buf: &[u8]) -> Option<(u64, usize)> {
    let mut n = 0u64;
    for (i, &b) in buf.iter().enumerate().take(10) {
        n |= u64::from(b & 0x7F).checked_shl(7 * i as u32)?;
        if b & 0x80 == 0 {
            return Some((n, i + 1));
        }
    }
    None
}

/// The number of bytes `put_varint` writes for `n`.
pub(crate) fn varint_len(n: u64) -> usize {
    (64 - (n | 1).leading_zeros() as usize).div_ceil(7)
}

/// Brings the length of `out` to an even number.
pub(crate) fn pad(out: &mut Vec<u8>) {
    if out.len() & 1 == 1 {
        out.push(0);
    }
}

fn put_short(out: &mut Vec<u8>, v: i16) {
    let v = v as u16 & 0x0FFF;
    out.extend_from_slice(&[(v >> 8) as u8, v as u8]);
}

pub(crate) fn put_int(out: &mut Vec<u8>, v: i64) {
    if (-2048..2048).contains(&v) {
        return put_short(out, v as i16);
    }
    let n = (1..8).find(|n| (v << (64 - 8 * n)) >> (64 - 8 * n) == v).unwrap_or(8);
    out.push(TAG_INT << 4 | (n as u8 - 1));
    out.extend_from_slice(&v.to_le_bytes()[..n]);
    pad(out);
}

pub(crate) fn put_uint(out: &mut Vec<u8>, v: u64) {
    if v < 2048 {
        return put_short(out, v as i16);
    }
    let n = (64 - v.leading_zeros() as usize).div_ceil(8);
    out.push(TAG_INT << 4 | 0x08 | (n as u8 - 1));
    out.extend_from_slice(&v.to_le_bytes()[..n]);
    pad(out);
}

pub(crate) fn put_f32(out: &mut Vec<u8>, v: f32) {
    out.extend_from_slice(&[FLOAT_32, 0]);
    out.extend_from_slice(&v.to_le_bytes());
}

pub(crate) fn put_f64(out: &mut Vec<u8>, v: f64) {
    // A double that a 32-bit float holds exactly takes 4 bytes, and stays a double.
    if f64::from(v as f32).to_bits() == v.to_bits() {
        out.extend_from_slice(&[FLOAT_32_DOUBLE, 0]);
        out.extend_from_slice(&(v as f32).to_le_bytes());
    } else {
        out.extend_from_slice(&[FLOAT_64, 0]);
        out.extend_from_slice(&v.to_le_bytes());
    }
}

/// A string (`TAG_STRING`) or binary data (`TAG_DATA`).
pub(crate) fn put_bytes(out: &mut Vec<u8>, tag: u8, bytes: &[u8]) {
    out.reserve(bytes.len() + 4);
    put_bytes_header(out, tag, bytes.len());
    out.extend_from_slice(bytes);
    pad(out);
}

/// What comes before the `len` bytes of a string or of binary data.
pub(crate) fn put_bytes_header(out: &mut Vec<u8>, tag: u8, len: usize) {
    if len < 15 {
        out.push(tag << 4 | len as u8);
    } else {
        out.push(tag << 4 | 15);
        put_varint(out, len as u64);
    }
}

/// The offset of a string's or data's bytes from the start of its value.
pub(crate) fn bytes_offset(len: usize) -> usize {
    if len < 15 { 1 } else { 1 + varint_len(len as u64) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_round_trip() {
        for n in [0, 1, 127, 128, 300, 16_383, 16_384, u64::from(u32::MAX), u64::MAX] {
            let mut buf = Vec::new();
            put_varint(&mut buf, n);
            assert_eq!(buf.len(), varint_len(n));
            assert_eq!(get_varint(&buf), Some((n, buf.len())));
        }
        assert_eq!(get_varint(&[0x80; 11]), None);
    }

    #[test]
    fn integers_take_the_fewest_bytes() {
        let bytes = |v: i64| {
            let mut out = Vec::new();
            put_int(&mut out, v);
            out
        };
        assert_eq!(bytes(123), [0x00, 0x7B]);
        assert_eq!(bytes(-1), [0x0F, 0xFF]);
        assert_eq!(bytes(-2048), [0x08, 0x00]);
        assert_eq!(bytes(2048), [0x11, 0x00, 0x08, 0x00]);
        assert_eq!(bytes(-2049), [0x11, 0xFF, 0xF7, 0x00]);
        assert_eq!(bytes(i64::MIN).len(), 10);
        let mut out = Vec::new();
        put_uint(&mut out, 1_234_567_890);
        assert_eq!(out, [0x1B, 0xD2, 0x02, 0x96, 0x49, 0x00]);
    }
}
