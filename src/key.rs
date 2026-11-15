//! Values as keys of an ordered store: bytes that sort as the values do.
//!
//! The bytes of a Fleece document do not sort in any useful way, and a store such as an LSM
//! tree orders its keys by their bytes. [`encode_key`] writes a value so that comparing two
//! encodings byte by byte gives the order of the two values, which is what an index over a
//! field of stored documents is made of: the encoded field first, the key of the document
//! after it.
//!
//! # The order
//!
//! The order is the one SurrealDB gives its values (`surrealdb_types::Value`), which the
//! tests of this crate compare it with for every type.
//!
//! Values of different types sort by type: none, null, bool, number, string, duration,
//! datetime, uuid, array, set, object, geometry, bytes, table and record, file, range,
//! regex, date, time, timestamp, interval, and last the vector, which SurrealDB has not.
//!
//! - **Numbers** of every kind sort together by what they are worth, so `1i8`, `1` and `1.0`
//!   are one key. The layout is that of SurrealDB's `BigDecimal::encode_ordered`: a sign
//!   byte, the power of ten of the first digit, a byte for each decimal digit, a terminator,
//!   all but the sign byte complemented for a negative number. A float takes part with its
//!   exact value. The infinities sort below and above all numbers, and NaN last.
//! - **Strings and bytes** sort by their bytes; **arrays** item by item, the shorter first;
//!   an **object** by its fields in the order of their names.
//! - A **set** sorts as its members in order, a member that is there twice counting once,
//!   whatever order they were given in.
//! - A **range** sorts by its start and then its end; of two bounds one that is not there
//!   comes first, then every bound taken in by its value, then every bound left out.
//! - An **interval** sorts by its length with a month of 30 days and a day of 24 hours,
//!   and two intervals of one length are one key.
//! - A **geometry** sorts by its kind, then by all its coordinates in a row (a polygon's
//!   inner rings before its outer one), then by how they are grouped into rings.
//! - A **table** sorts by its name just before the **records** of the table. SurrealDB
//!   calls a table equal to each of its records, which bytes cannot be; every other
//!   comparison of a table and a record is the same.
//!
//! An encoding is for comparing, not for reading back: it does not keep the kind of a
//! number, and no decoder is given.

use std::ops::Bound;

use crate::value::*;

const NONE: u8 = 0x01;
const NULL: u8 = 0x02;
const BOOL: u8 = 0x03;
const NUMBER: u8 = 0x04;
const STRING: u8 = 0x05;
const DURATION: u8 = 0x06;
const DATETIME: u8 = 0x07;
const UUID: u8 = 0x08;
const ARRAY: u8 = 0x09;
const SET: u8 = 0x0A;
const OBJECT: u8 = 0x0B;
const GEOMETRY: u8 = 0x0C;
const BYTES: u8 = 0x0D;
/// A table, and a record of it.
const TABLE: u8 = 0x0E;
const FILE: u8 = 0x10;
const RANGE: u8 = 0x11;
const REGEX: u8 = 0x12;
const DATE: u8 = 0x13;
const TIME: u8 = 0x14;
const TIMESTAMP: u8 = 0x15;
const INTERVAL: u8 = 0x16;
const VECTOR: u8 = 0x17;

/// What ends a row of items. Every item starts with a byte above it, so of two rows that
/// agree as far as the shorter one goes, the shorter one sorts first.
const END: u8 = 0x00;
/// What starts an item that has no type byte of its own to start with: an entry of an
/// object, a point, a count.
const ENTRY: u8 = 0x01;

/// The bytes that sort as `value` does among all values.
pub fn encode_key(value: &Value) -> Vec<u8> {
    let mut out = Vec::new();
    encode_key_into(value, &mut out);
    out
}

/// Appends the bytes of [`encode_key`] to `out`, as a key of several parts is made.
pub fn encode_key_into(value: &Value, out: &mut Vec<u8>) {
    match value {
        Value::None => out.push(NONE),
        Value::Null => out.push(NULL),
        Value::Bool(v) => out.extend_from_slice(&[BOOL, u8::from(*v)]),
        Value::Number(v) => {
            out.push(NUMBER);
            number(v, out);
        }
        Value::String(v) => tagged_text(STRING, v.as_bytes(), out),
        Value::Bytes(v) => tagged_text(BYTES, v, out),
        Value::Regex(v) => tagged_text(REGEX, v.as_bytes(), out),
        Value::Duration(v) => {
            out.push(DURATION);
            out.extend_from_slice(&v.secs.to_be_bytes());
            out.extend_from_slice(&v.nanos.to_be_bytes());
        }
        Value::Datetime(v) => instant(DATETIME, v.secs, v.nanos, out),
        Value::Timestamp(v) => instant(TIMESTAMP, v.secs, v.nanos, out),
        Value::Uuid(v) => {
            out.push(UUID);
            out.extend_from_slice(&v.0);
        }
        Value::Date(v) => {
            out.push(DATE);
            out.extend_from_slice(&signed(i64::from(v.days)));
        }
        Value::Time(v) => {
            out.push(TIME);
            out.extend_from_slice(&v.nanos.to_be_bytes());
        }
        Value::Interval(v) => {
            // A month of 30 days and a day of 24 hours, as SurrealDB compares intervals.
            const DAY: i128 = 24 * 60 * 60 * 1_000_000;
            let micros =
                i128::from(v.months) * 30 * DAY + i128::from(v.days) * DAY + i128::from(v.micros);
            out.push(INTERVAL);
            out.extend_from_slice(&(micros as u128 ^ 1 << 127).to_be_bytes());
        }
        Value::Array(items) => {
            out.push(ARRAY);
            for item in items {
                encode_key_into(item, out);
            }
            out.push(END);
        }
        Value::Set(items) => {
            // In order and each once, whatever was given: the set SurrealDB keeps.
            let mut members: Vec<Vec<u8>> = items.iter().map(encode_key).collect();
            members.sort_unstable();
            members.dedup();
            out.push(SET);
            members.iter().for_each(|member| out.extend_from_slice(member));
            out.push(END);
        }
        Value::Object(fields) => {
            out.push(OBJECT);
            object(fields, out);
        }
        Value::Table(v) => {
            tagged_text(TABLE, v.as_bytes(), out);
            out.push(END);
        }
        Value::RecordId(v) => {
            tagged_text(TABLE, v.table.as_bytes(), out);
            out.push(ENTRY);
            record_key(&v.key, out);
        }
        Value::File(v) => {
            out.push(FILE);
            text(v.bucket.as_bytes(), out);
            text(v.key.as_bytes(), out);
        }
        Value::Range(v) => {
            out.push(RANGE);
            bound(&v.start, out, encode_key_into);
            bound(&v.end, out, encode_key_into);
        }
        Value::Geometry(v) => {
            out.push(GEOMETRY);
            geometry(v, out);
        }
        Value::Vector(v) => {
            out.push(VECTOR);
            for element in v.to_f32() {
                out.push(ENTRY);
                out.extend_from_slice(&float(f64::from(element)));
            }
            out.push(END);
        }
    }
}

/// A signed integer as 8 bytes that sort as the integers do: big-endian, the sign bit
/// turned over.
fn signed(value: i64) -> [u8; 8] {
    (value as u64 ^ 1 << 63).to_be_bytes()
}

fn instant(tag: u8, secs: i64, nanos: u32, out: &mut Vec<u8>) {
    out.push(tag);
    out.extend_from_slice(&signed(secs));
    out.extend_from_slice(&nanos.to_be_bytes());
}

/// Bytes of any length: a zero byte is written as `00 FF`, and `00 00` ends them, so that
/// bytes that are the start of other bytes sort first.
fn text(bytes: &[u8], out: &mut Vec<u8>) {
    for &byte in bytes {
        out.push(byte);
        if byte == 0 {
            out.push(0xFF);
        }
    }
    out.extend_from_slice(&[0, 0]);
}

fn tagged_text(tag: u8, bytes: &[u8], out: &mut Vec<u8>) {
    out.push(tag);
    text(bytes, out);
}

fn object(fields: &Object, out: &mut Vec<u8>) {
    for (key, value) in fields {
        out.push(ENTRY);
        text(key.as_bytes(), out);
        encode_key_into(value, out);
    }
    out.push(END);
}

/// One end of a range: none first, then every bound taken in by its value, then every
/// bound left out by its value, for a start and for an end alike.
fn bound<T>(bound: &Bound<T>, out: &mut Vec<u8>, put: impl Fn(&T, &mut Vec<u8>)) {
    match bound {
        Bound::Unbounded => out.push(0x01),
        Bound::Included(value) => {
            out.push(0x02);
            put(value, out);
        }
        Bound::Excluded(value) => {
            out.push(0x03);
            put(value, out);
        }
    }
}

fn record_key(key: &RecordIdKey, out: &mut Vec<u8>) {
    match key {
        RecordIdKey::Number(v) => {
            out.push(0x01);
            out.extend_from_slice(&signed(*v));
        }
        RecordIdKey::String(v) => tagged_text(0x02, v.as_bytes(), out),
        RecordIdKey::Uuid(v) => {
            out.push(0x03);
            out.extend_from_slice(&v.0);
        }
        RecordIdKey::Array(items) => {
            out.push(0x04);
            for item in items {
                encode_key_into(item, out);
            }
            out.push(END);
        }
        RecordIdKey::Object(fields) => {
            out.push(0x05);
            object(fields, out);
        }
        RecordIdKey::Range(range) => {
            out.push(0x06);
            bound(&range.start, out, record_key);
            bound(&range.end, out, record_key);
        }
    }
}

/// A double as 8 bytes that sort as `f64::total_cmp` does, the two zeros being one.
fn float(value: f64) -> [u8; 8] {
    let bits = if value == 0.0 { 0 } else { value.to_bits() };
    // A negative double sorts by its bits backwards, a positive one above every negative.
    (if bits >> 63 == 1 { !bits } else { bits ^ 1 << 63 }).to_be_bytes()
}

/// The coordinates of a geometry in a row, which is what it sorts by first.
fn coordinates<'p>(points: impl Iterator<Item = &'p Point>, out: &mut Vec<u8>) {
    for point in points {
        out.push(ENTRY);
        out.extend_from_slice(&float(point.x));
        out.extend_from_slice(&float(point.y));
    }
    out.push(END);
}

/// Counts in a row: how the coordinates are grouped, which geometries of the same
/// coordinates sort by.
fn counts(counts: impl Iterator<Item = usize>, out: &mut Vec<u8>) {
    for count in counts {
        out.push(ENTRY);
        out.extend_from_slice(&(count as u64).to_be_bytes());
    }
    out.push(END);
}

/// The points of a polygon, its inner rings before its outer one.
fn polygon_points(polygon: &Polygon) -> impl Iterator<Item = &Point> {
    polygon.interiors.iter().chain(std::iter::once(&polygon.exterior)).flatten()
}

/// The number of inner rings of a polygon, the length of each, and that of the outer one.
fn polygon_shape(polygon: &Polygon) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(polygon.interiors.len())
        .chain(polygon.interiors.iter().map(Vec::len))
        .chain(std::iter::once(polygon.exterior.len()))
}

fn geometry(shape: &Geometry, out: &mut Vec<u8>) {
    match shape {
        Geometry::Point(point) => {
            out.push(0x01);
            coordinates(std::iter::once(point), out);
        }
        Geometry::Line(line) => {
            out.push(0x02);
            coordinates(line.iter(), out);
        }
        Geometry::Polygon(polygon) => {
            out.push(0x03);
            coordinates(polygon_points(polygon), out);
            counts(polygon_shape(polygon), out);
        }
        Geometry::MultiPoint(points) => {
            out.push(0x04);
            coordinates(points.iter(), out);
        }
        Geometry::MultiLine(lines) => {
            out.push(0x05);
            coordinates(lines.iter().flatten(), out);
            counts(lines.iter().map(Vec::len), out);
        }
        Geometry::MultiPolygon(polygons) => {
            out.push(0x06);
            coordinates(polygons.iter().flat_map(polygon_points), out);
            let shapes = polygons.iter().flat_map(polygon_shape);
            counts(std::iter::once(polygons.len()).chain(shapes), out);
        }
        Geometry::Collection(geometries) => {
            out.push(0x07);
            for inner in geometries {
                out.push(ENTRY);
                geometry(inner, out);
            }
            out.push(END);
        }
    }
}

const NEGATIVE_INFINITY: u8 = 0x00;
const NEGATIVE: u8 = 0x01;
const ZERO: u8 = 0x02;
const POSITIVE: u8 = 0x03;
const INFINITY: u8 = 0x04;
const NAN: u8 = 0x05;

/// A number as its sign, the power of ten of its first digit, and its decimal digits.
fn number(number: &Number, out: &mut Vec<u8>) {
    let integer = |negative: bool, magnitude: u64, out: &mut Vec<u8>| {
        decimal(negative, &magnitude.to_string(), 0, out);
    };
    let whole = |value: i64, out: &mut Vec<u8>| integer(value < 0, value.unsigned_abs(), out);
    match number {
        Number::Int(v) => whole(*v, out),
        Number::I8(v) => whole(i64::from(*v), out),
        Number::I16(v) => whole(i64::from(*v), out),
        Number::I32(v) => whole(i64::from(*v), out),
        Number::U8(v) => integer(false, u64::from(*v), out),
        Number::U16(v) => integer(false, u64::from(*v), out),
        Number::U32(v) => integer(false, u64::from(*v), out),
        Number::U64(v) => integer(false, *v, out),
        Number::Float(v) => double(*v, out),
        Number::F32(v) => double(f64::from(*v), out),
        Number::F16(v) => double(v.to_f64(), out),
        Number::Decimal(v) => {
            decimal(v.is_negative(), &v.mantissa().to_string(), i64::from(v.scale()), out);
        }
        Number::BigDecimal(v) => {
            let text = v.to_string();
            let digits: String = text.chars().filter(char::is_ascii_digit).collect();
            decimal(v.is_negative(), &digits, i64::from(v.scale()), out);
        }
    }
}

/// The number `digits * 10^-scale`, negated when `negative`. `digits` are decimal digits
/// with any zeros before and after them.
fn decimal(negative: bool, digits: &str, scale: i64, out: &mut Vec<u8>) {
    let significant = digits.trim_start_matches('0');
    if significant.is_empty() {
        return out.push(ZERO);
    }
    // The power of ten of the first digit: 150 with scale 2 is 1.50, whose first digit
    // counts ones.
    let exponent = significant.len() as i64 - 1 - scale;
    let start = out.len() + 1;
    out.push(if negative { NEGATIVE } else { POSITIVE });
    out.extend_from_slice(&signed(exponent));
    // A digit is a byte from 1 to 10, above the terminator, so that of two numbers that
    // agree as far as the shorter goes, the one with more digits is the greater.
    out.extend(significant.trim_end_matches('0').bytes().map(|digit| digit - b'0' + 1));
    out.push(0x00);
    if negative {
        // Turned over, the greater magnitude sorts first, as the lesser number it is.
        for byte in &mut out[start..] {
            *byte = !*byte;
        }
    }
}

/// A double by its exact value, which is `mantissa * 2^exponent`.
fn double(value: f64, out: &mut Vec<u8>) {
    if value.is_nan() {
        return out.push(NAN);
    }
    if value.is_infinite() {
        return out.push(if value < 0.0 { NEGATIVE_INFINITY } else { INFINITY });
    }
    let bits = value.to_bits();
    let biased = (bits >> 52 & 0x7FF) as i64;
    let fraction = bits & ((1 << 52) - 1);
    // A subnormal has no hidden bit and the exponent of the smallest normal.
    let (mantissa, exponent) = match biased {
        0 => (fraction, -1074),
        _ => (fraction | 1 << 52, biased - 1075),
    };
    let mut limbs = vec![mantissa as u32, (mantissa >> 32) as u32];
    // Times 2^exponent when that is whole. Otherwise times 5^-exponent, which is the same
    // digits with the point moved -exponent places: m / 2^k = m * 5^k / 10^k.
    let (factor, mut times, scale) = match exponent {
        0.. => (2, exponent, 0),
        _ => (5, -exponent, -exponent),
    };
    // 2^31 and 5^13 are the greatest powers that fit a 32-bit digit.
    let (step, power) = if factor == 2 { (31, 1 << 31) } else { (13, 1_220_703_125) };
    while times >= step {
        multiply(&mut limbs, power);
        times -= step;
    }
    multiply(&mut limbs, (factor as u32).pow(times as u32));
    decimal(value.is_sign_negative(), &decimal_digits(limbs), scale, out);
}

/// Multiplies a number of 32-bit digits, the lowest first, by a 32-bit number.
fn multiply(limbs: &mut Vec<u32>, by: u32) {
    let mut carry = 0;
    for limb in limbs.iter_mut() {
        let product = u64::from(*limb) * u64::from(by) + carry;
        *limb = product as u32;
        carry = product >> 32;
    }
    if carry != 0 {
        limbs.push(carry as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(number: Number) -> Vec<u8> {
        encode_key(&Value::Number(number))
    }

    #[test]
    fn numbers_are_laid_out_as_documented() {
        // 1.5: positive, first digit counts ones, digits 1 and 5.
        let one_and_a_half = [NUMBER, POSITIVE, 0x80, 0, 0, 0, 0, 0, 0, 0, 2, 6, 0];
        assert_eq!(key(Number::Float(1.5)), one_and_a_half);
        assert_eq!(key(Number::Decimal("1.50".parse().unwrap())), one_and_a_half);
        assert_eq!(key(Number::BigDecimal("1.5000".parse().unwrap())), one_and_a_half);
        assert_eq!(key(Number::F32(1.5)), one_and_a_half);
        // -1.5: the same with everything after the sign byte turned over.
        let minus = [NUMBER, NEGATIVE, 0x7F, 255, 255, 255, 255, 255, 255, 255, 253, 249, 255];
        assert_eq!(key(Number::Float(-1.5)), minus);
        assert_eq!(key(Number::Int(0)), [NUMBER, ZERO]);
        assert_eq!(key(Number::Float(-0.0)), [NUMBER, ZERO]);
        assert_eq!(key(Number::Int(100)), key(Number::U8(100)));
        assert_eq!(key(Number::Int(100)), [NUMBER, POSITIVE, 0x80, 0, 0, 0, 0, 0, 0, 2, 2, 0]);
        // A double is its exact value, which for 0.1 is a little above the decimal 0.1.
        assert!(key(Number::Float(0.1)) > key(Number::Decimal("0.1".parse().unwrap())));
        assert_eq!(key(Number::Float(0.5)), key(Number::Decimal("0.5".parse().unwrap())));
        assert_eq!(
            key(Number::Float(f64::from_bits(1))),
            key(Number::BigDecimal(format!("0.{}", exact_smallest()).parse().unwrap()))
        );
    }

    /// The digits after the point of the smallest double above zero, 2^-1074, in full.
    fn exact_smallest() -> String {
        let mut limbs = vec![1];
        for _ in 0..1074 {
            multiply(&mut limbs, 5);
        }
        format!("{:0>1074}", decimal_digits(limbs))
    }
}
