//! An owned value with the types of SurrealDB.
//!
//! The types mirror `surrealdb_types::Value` (4.0) variant by variant and hold plain data, so
//! that a conversion to and from that crate is a field-by-field copy. They do not check what
//! SurrealDB checks (a calendar date in range, a regular expression that compiles).

use std::collections::BTreeMap;
use std::fmt::{self, Display, Formatter};
use std::ops::Bound;
use std::str::FromStr;

/// An object: its fields by name.
pub type Object = BTreeMap<String, Value>;

/// A value of any type SurrealDB has.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Value {
    /// The absence of a value.
    #[default]
    None,
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Bytes(Vec<u8>),
    Duration(Duration),
    Datetime(Datetime),
    Uuid(Uuid),
    Geometry(Geometry),
    /// A table, by name.
    Table(String),
    RecordId(RecordId),
    File(File),
    Range(Box<Range>),
    /// A regular expression, as its pattern.
    Regex(String),
    Array(Vec<Value>),
    Object(Object),
    /// A set, as its members in the order given. Keeping them unique and sorted is up to
    /// the caller: the order of values of SurrealDB is not known here.
    Set(Vec<Value>),
    Date(Date),
    Time(Time),
    Timestamp(Timestamp),
    Interval(Interval),
}

/// A number, of the kind it was written as.
#[derive(Clone, Debug, PartialEq)]
pub enum Number {
    Int(i64),
    Float(f64),
    Decimal(Decimal),
    I8(i8),
    U8(u8),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    U64(u64),
    F16(F16),
    F32(f32),
    BigDecimal(BigDecimal),
}

/// A 16-bit IEEE 754 float, as its bits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct F16(pub u16);

impl F16 {
    /// The value as a double, which holds every `f16` exactly.
    pub fn to_f64(self) -> f64 {
        let sign = if self.0 & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exponent = i32::from(self.0 >> 10 & 0x1F);
        let fraction = f64::from(self.0 & 0x3FF);
        sign * match exponent {
            0 => fraction * 2f64.powi(-24),
            31 if fraction == 0.0 => f64::INFINITY,
            31 => f64::NAN,
            _ => (1024.0 + fraction) * 2f64.powi(exponent - 25),
        }
    }

    /// The `f16` equal to `value`, when there is one. A NaN gives the quiet NaN.
    pub fn from_f64(value: f64) -> Option<Self> {
        let sign = if value.is_sign_negative() { 0x8000 } else { 0 };
        if value.is_nan() {
            return Some(Self(0x7E00));
        }
        if value.is_infinite() {
            return Some(Self(sign | 0x7C00));
        }
        if value == 0.0 {
            return Some(Self(sign));
        }
        let bits = value.to_bits();
        let exponent = (bits >> 52 & 0x7FF) as i32 - 1023;
        let mantissa = bits & ((1 << 52) - 1);
        // How many low bits of the mantissa an f16 of this magnitude cannot hold.
        let (dropped, biased) = match exponent {
            -14..=15 => (42, (exponent + 15) as u16),
            -24..=-15 => ((28 - exponent) as u32, 0),
            _ => return None,
        };
        let significand = if biased == 0 { mantissa | 1 << 52 } else { mantissa };
        if significand & ((1 << dropped) - 1) != 0 {
            return None;
        }
        Some(Self(sign | biased << 10 | (significand >> dropped) as u16))
    }
}

/// A decimal number of up to 96 bits and a scale of up to 28, as `rust_decimal` holds it.
///
/// The 16 bytes are those of `rust_decimal::Decimal::serialize`: the flags (the scale in
/// bits 16 to 23, the sign in bit 31), then the low, middle and high 32 bits, little-endian.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Decimal([u8; 16]);

impl Decimal {
    /// The greatest scale: the number of digits after the point.
    pub const MAX_SCALE: u8 = 28;

    /// `mantissa * 10^-scale`, negated when `negative`. `None` when the mantissa takes more
    /// than 96 bits or the scale is above [`Decimal::MAX_SCALE`].
    pub fn new(mantissa: u128, scale: u8, negative: bool) -> Option<Self> {
        if mantissa >> 96 != 0 || scale > Self::MAX_SCALE {
            return None;
        }
        let mut bytes = [0; 16];
        bytes[2] = scale;
        bytes[3] = if negative { 0x80 } else { 0 };
        bytes[4..].copy_from_slice(&mantissa.to_le_bytes()[..12]);
        Some(Self(bytes))
    }

    /// The decimal these bytes hold, when they are bytes a decimal has.
    pub fn from_bytes(bytes: [u8; 16]) -> Option<Self> {
        let well_formed =
            bytes[0] == 0 && bytes[1] == 0 && bytes[2] <= Self::MAX_SCALE && bytes[3] & 0x7F == 0;
        well_formed.then_some(Self(bytes))
    }

    pub fn to_bytes(self) -> [u8; 16] {
        self.0
    }

    pub fn mantissa(&self) -> u128 {
        let mut bytes = [0; 16];
        bytes[..12].copy_from_slice(&self.0[4..]);
        u128::from_le_bytes(bytes)
    }

    pub fn scale(&self) -> u8 {
        self.0[2]
    }

    pub fn is_negative(&self) -> bool {
        self.0[3] != 0
    }
}

impl Display for Decimal {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let scale = usize::from(self.scale());
        let digits = format!("{:0>width$}", self.mantissa(), width = scale + 1);
        let (whole, fraction) = digits.split_at(digits.len() - scale);
        let sign = if self.is_negative() { "-" } else { "" };
        let point = if scale > 0 { "." } else { "" };
        write!(f, "{sign}{whole}{point}{fraction}")
    }
}

/// The parts of a decimal number written as `-123.45`: its sign, whole part and fraction.
fn decimal_parts(text: &str) -> Option<(bool, &str, &str)> {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(digits) => (true, digits),
        None => (false, text),
    };
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) if !fraction.is_empty() => (whole, fraction),
        Some(_) => return None,
        None => (digits, ""),
    };
    let all_digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    (!whole.is_empty() && all_digits(whole) && all_digits(fraction))
        .then_some((negative, whole, fraction))
}

/// The text is not a number of the type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseError;

impl Display for ParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("the text is not a value of the type")
    }
}

impl std::error::Error for ParseError {}

impl FromStr for Decimal {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Self, ParseError> {
        let (negative, whole, fraction) = decimal_parts(text).ok_or(ParseError)?;
        let mantissa = format!("{whole}{fraction}").parse().map_err(|_| ParseError)?;
        let scale = u8::try_from(fraction.len()).map_err(|_| ParseError)?;
        Self::new(mantissa, scale, negative).ok_or(ParseError)
    }
}

/// A decimal number of any size, as the text it was written as: an optional `-`, digits, and
/// optionally a point and more digits. The text keeps the scale (`1.50` is not `1.5`), which
/// is what SurrealDB's `bigdecimal` does.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BigDecimal(String);

impl BigDecimal {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BigDecimal {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Self, ParseError> {
        decimal_parts(text).map(|_| Self(text.to_owned())).ok_or(ParseError)
    }
}

impl Display for BigDecimal {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A span of time: whole seconds and the nanoseconds of the last one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Duration {
    pub secs: u64,
    pub nanos: u32,
}

/// A point in time: seconds since 1970-01-01T00:00:00Z and nanoseconds of that second.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Datetime {
    pub secs: i64,
    pub nanos: u32,
}

/// A date and time of day without a time zone, counted as [`Datetime`] counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp {
    pub secs: i64,
    pub nanos: u32,
}

/// A calendar date: days since 1970-01-01.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Date {
    pub days: i32,
}

/// A time of day: nanoseconds since midnight, up to `24:00:00`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Time {
    pub nanos: u64,
}

impl Time {
    /// The nanoseconds of a day: the latest time of day.
    pub const MAX_NANOS: u64 = 86_400_000_000_000;
}

/// A calendar-aware interval.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Interval {
    pub months: i32,
    pub days: i32,
    pub micros: i64,
}

/// A UUID, as its 16 bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Uuid(pub [u8; 16]);

impl Display for Uuid {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for (i, byte) in self.0.iter().enumerate() {
            if matches!(i, 4 | 6 | 8 | 10) {
                f.write_str("-")?;
            }
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl FromStr for Uuid {
    type Err = ParseError;

    fn from_str(text: &str) -> Result<Self, ParseError> {
        let groups: Vec<&str> = text.split('-').collect();
        if groups.iter().map(|group| group.len()).ne([8, 4, 4, 4, 12]) {
            return Err(ParseError);
        }
        let hex = groups.concat();
        let mut bytes = [0; 16];
        for (byte, pair) in bytes.iter_mut().zip(hex.as_bytes().chunks(2)) {
            let pair = std::str::from_utf8(pair).map_err(|_| ParseError)?;
            if !pair.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(ParseError);
            }
            *byte = u8::from_str_radix(pair, 16).map_err(|_| ParseError)?;
        }
        Ok(Self(bytes))
    }
}

/// A file in a bucket.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct File {
    pub bucket: String,
    pub key: String,
}

/// A record: a table and the key of the record in it.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordId {
    pub table: String,
    pub key: RecordIdKey,
}

/// The key of a record.
#[derive(Clone, Debug, PartialEq)]
pub enum RecordIdKey {
    Number(i64),
    String(String),
    Uuid(Uuid),
    Array(Vec<Value>),
    Object(Object),
    Range(Box<RecordIdKeyRange>),
}

/// A range of record keys.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordIdKeyRange {
    pub start: Bound<RecordIdKey>,
    pub end: Bound<RecordIdKey>,
}

impl From<RecordIdKey> for Value {
    /// The key as a value: a range of keys becomes a range of values.
    fn from(key: RecordIdKey) -> Self {
        match key {
            RecordIdKey::Number(v) => Value::Number(Number::Int(v)),
            RecordIdKey::String(v) => Value::String(v),
            RecordIdKey::Uuid(v) => Value::Uuid(v),
            RecordIdKey::Array(v) => Value::Array(v),
            RecordIdKey::Object(v) => Value::Object(v),
            RecordIdKey::Range(range) => Value::Range(Box::new(Range {
                start: range.start.map(Value::from),
                end: range.end.map(Value::from),
            })),
        }
    }
}

impl TryFrom<Value> for RecordIdKey {
    type Error = ParseError;

    /// The value as a record key, when it is of a type a key has.
    fn try_from(value: Value) -> Result<Self, ParseError> {
        let bound = |bound: Bound<Value>| -> Result<Bound<Self>, ParseError> {
            Ok(match bound {
                Bound::Included(v) => Bound::Included(Self::try_from(v)?),
                Bound::Excluded(v) => Bound::Excluded(Self::try_from(v)?),
                Bound::Unbounded => Bound::Unbounded,
            })
        };
        Ok(match value {
            Value::Number(Number::Int(v)) => Self::Number(v),
            Value::String(v) => Self::String(v),
            Value::Uuid(v) => Self::Uuid(v),
            Value::Array(v) => Self::Array(v),
            Value::Object(v) => Self::Object(v),
            Value::Range(range) => Self::Range(Box::new(RecordIdKeyRange {
                start: bound(range.start)?,
                end: bound(range.end)?,
            })),
            _ => return Err(ParseError),
        })
    }
}

/// A range of values.
#[derive(Clone, Debug, PartialEq)]
pub struct Range {
    pub start: Bound<Value>,
    pub end: Bound<Value>,
}

/// A point of a geometry.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// A polygon: its outer ring and the rings of its holes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Polygon {
    pub exterior: Vec<Point>,
    pub interiors: Vec<Vec<Point>>,
}

/// A geometric shape.
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    Point(Point),
    Line(Vec<Point>),
    Polygon(Polygon),
    MultiPoint(Vec<Point>),
    MultiLine(Vec<Vec<Point>>),
    MultiPolygon(Vec<Polygon>),
    Collection(Vec<Geometry>),
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Number(Number::Int(v))
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Number(Number::Float(v))
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::String(v.to_owned())
    }
}

impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::String(v)
    }
}

impl From<Vec<Value>> for Value {
    fn from(v: Vec<Value>) -> Self {
        Value::Array(v)
    }
}

impl From<Object> for Value {
    fn from(v: Object) -> Self {
        Value::Object(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_converts_exactly() {
        for bits in 0..=u16::MAX {
            let value = F16(bits).to_f64();
            if value.is_nan() {
                continue;
            }
            assert_eq!(F16::from_f64(value), Some(F16(bits)), "{bits:#06x}");
        }
        assert_eq!(F16(0x3E00).to_f64(), 1.5);
        assert_eq!(F16(0x0001).to_f64(), 2f64.powi(-24));
        assert_eq!(F16::from_f64(0.1), None);
        assert_eq!(F16::from_f64(65_536.0), None);
    }

    #[test]
    fn decimal_text() {
        for text in ["0", "-1.50", "0.001", "79228162514264337593543950335", "12.3400"] {
            assert_eq!(text.parse::<Decimal>().unwrap().to_string(), text);
        }
        assert_eq!("1.5".parse::<Decimal>().unwrap(), Decimal::new(15, 1, false).unwrap());
        for text in ["", "-", "1.", ".5", "1e3", "79228162514264337593543950336"] {
            assert!(text.parse::<Decimal>().is_err(), "{text}");
        }
        assert!("-0.000000000000000000000000000000001".parse::<BigDecimal>().is_ok());
    }

    #[test]
    fn uuid_text() {
        let text = "018f4a2b-7c3d-7e1f-9a0b-1c2d3e4f5a6b";
        assert_eq!(text.parse::<Uuid>().unwrap().to_string(), text);
        assert!("018f4a2b7c3d7e1f9a0b1c2d3e4f5a6b".parse::<Uuid>().is_err());
        assert!("018f4a2b-7c3d-7e1f-9a0b-1c2d3e4f5a6+".parse::<Uuid>().is_err());
    }
}
