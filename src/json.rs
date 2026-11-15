//! JSON to values and back.
//!
//! JSON has six types, so a value of any other type is written as an object with one field
//! whose name is `$` and the type: `{"$uuid": "018f4a2b-…"}`, `{"$u8": 7}`,
//! `{"$bytes": "3q2+7w=="}`. Reading such an object gives the typed value back, which makes
//! the conversion lossless both ways. A field of an object whose own name starts with `$`
//! is written with one more `$` in front, and read back without it.

use std::fmt::{self, Display, Formatter};
use std::ops::Bound;

use serde_json::{Map, Value as Json, json};

use crate::codec::{decode, encode};
use crate::error::{DecodeError, EncodeError};
use crate::value::*;

/// Why JSON could not be turned into a value, or a document into JSON.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JsonError {
    /// The text is not JSON.
    Syntax(String),
    /// An object `{"$name": …}` names a type that does not exist.
    UnknownType(String),
    /// The contents of an object `{"$name": …}` are not a value of the type it names.
    BadValue(String),
    /// The document was refused.
    Decode(DecodeError),
    /// The value could not be written.
    Encode(EncodeError),
}

impl Display for JsonError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Syntax(e) => write!(f, "not JSON: {e}"),
            Self::UnknownType(name) => write!(f, "unknown type ${name}"),
            Self::BadValue(name) => write!(f, "not a value of the type ${name}"),
            Self::Decode(e) => write!(f, "{e}"),
            Self::Encode(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for JsonError {}

/// The document of the value a JSON text stands for.
pub fn json_to_fleece(text: &str) -> Result<Vec<u8>, JsonError> {
    let json: Json = serde_json::from_str(text).map_err(|e| JsonError::Syntax(e.to_string()))?;
    encode(&from_json(&json)?).map_err(JsonError::Encode)
}

/// The root of a document as JSON text.
pub fn fleece_to_json(bytes: &[u8]) -> Result<String, JsonError> {
    Ok(to_json(&decode(bytes).map_err(JsonError::Decode)?).to_string())
}

fn typed(name: &str, payload: Json) -> Json {
    Json::Object(Map::from_iter([(format!("${name}"), payload)]))
}

/// A float as a JSON number, or as text when JSON has no number for it.
fn float_json(value: f64) -> Json {
    match serde_json::Number::from_f64(value) {
        Some(number) => Json::Number(number),
        None if value.is_nan() => "NaN".into(),
        None if value > 0.0 => "Infinity".into(),
        None => "-Infinity".into(),
    }
}

fn float_from(json: &Json) -> Option<f64> {
    match json {
        Json::Number(number) => number.as_f64(),
        Json::String(text) => match text.as_str() {
            "NaN" => Some(f64::NAN),
            "Infinity" => Some(f64::INFINITY),
            "-Infinity" => Some(f64::NEG_INFINITY),
            _ => None,
        },
        _ => None,
    }
}

/// The value as JSON.
pub fn to_json(value: &Value) -> Json {
    match value {
        Value::None => typed("none", Json::Null),
        Value::Null => Json::Null,
        Value::Bool(v) => json!(v),
        Value::Number(number) => match number {
            Number::Int(v) => json!(v),
            Number::Float(v) if v.is_finite() => float_json(*v),
            Number::Float(v) => typed("float", float_json(*v)),
            Number::F32(v) => typed("f32", float_json(f64::from(*v))),
            Number::F16(v) => typed("f16", float_json(v.to_f64())),
            Number::Decimal(v) => typed("decimal", json!(v.to_string())),
            Number::BigDecimal(v) => typed("bigdecimal", json!(v.as_str())),
            Number::I8(v) => typed("i8", json!(v)),
            Number::U8(v) => typed("u8", json!(v)),
            Number::I16(v) => typed("i16", json!(v)),
            Number::U16(v) => typed("u16", json!(v)),
            Number::I32(v) => typed("i32", json!(v)),
            Number::U32(v) => typed("u32", json!(v)),
            Number::U64(v) => typed("u64", json!(v)),
        },
        Value::String(v) => json!(v),
        Value::Bytes(v) => typed("bytes", json!(base64_encode(v))),
        Value::Duration(v) => typed("duration", json!({ "secs": v.secs, "nanos": v.nanos })),
        Value::Datetime(v) => typed("datetime", json!(format_instant(v.secs, v.nanos) + "Z")),
        Value::Timestamp(v) => typed("timestamp", json!(format_instant(v.secs, v.nanos))),
        Value::Date(v) => typed("date", json!(format_date(i64::from(v.days)))),
        Value::Time(v) => typed("time", json!(format_clock(v.nanos))),
        Value::Interval(v) => {
            typed("interval", json!({ "months": v.months, "days": v.days, "micros": v.micros }))
        }
        Value::Uuid(v) => typed("uuid", json!(v.to_string())),
        Value::Table(v) => typed("table", json!(v)),
        Value::Regex(v) => typed("regex", json!(v)),
        Value::RecordId(v) => {
            let key = to_json(&Value::from(v.key.clone()));
            typed("record", json!({ "table": v.table, "key": key }))
        }
        Value::File(v) => typed("file", json!({ "bucket": v.bucket, "key": v.key })),
        Value::Range(v) => {
            let bound = |name: &str, bound: &Bound<Value>| match bound {
                Bound::Included(v) => Some((name.to_owned(), json!({ "included": to_json(v) }))),
                Bound::Excluded(v) => Some((name.to_owned(), json!({ "excluded": to_json(v) }))),
                Bound::Unbounded => None,
            };
            let bounds = [bound("start", &v.start), bound("end", &v.end)];
            typed("range", Json::Object(bounds.into_iter().flatten().collect()))
        }
        Value::Array(items) => Json::Array(items.iter().map(to_json).collect()),
        Value::Set(items) => typed("set", Json::Array(items.iter().map(to_json).collect())),
        Value::Object(fields) => {
            let field = |(key, value): (&String, &Value)| {
                let key = if key.starts_with('$') { format!("${key}") } else { key.clone() };
                (key, to_json(value))
            };
            Json::Object(fields.iter().map(field).collect())
        }
        Value::Geometry(v) => typed("geometry", geometry_json(v)),
    }
}

/// The value JSON stands for.
pub fn from_json(json: &Json) -> Result<Value, JsonError> {
    Ok(match json {
        Json::Null => Value::Null,
        Json::Bool(v) => Value::Bool(*v),
        Json::Number(number) => Value::Number(match (number.as_i64(), number.as_u64()) {
            (Some(v), _) => Number::Int(v),
            (None, Some(v)) => Number::U64(v),
            (None, None) => Number::Float(number.as_f64().unwrap_or(f64::NAN)),
        }),
        Json::String(v) => Value::String(v.clone()),
        Json::Array(items) => Value::Array(items.iter().map(from_json).collect::<Result<_, _>>()?),
        Json::Object(fields) => {
            if fields.len() == 1
                && let Some((key, payload)) = fields.iter().next()
                && let Some(name) = key.strip_prefix('$')
                && !name.starts_with('$')
            {
                return typed_from(name, payload);
            }
            let field = |(key, value): (&String, &Json)| {
                let key = if key.starts_with("$$") { &key[1..] } else { key.as_str() };
                from_json(value).map(|value| (key.to_owned(), value))
            };
            Value::Object(fields.iter().map(field).collect::<Result<_, _>>()?)
        }
    })
}

fn field<'j>(json: &'j Json, name: &str) -> Option<&'j Json> {
    json.as_object()?.get(name)
}

fn int_field<T: TryFrom<i64>>(json: &Json, name: &str) -> Option<T> {
    T::try_from(field(json, name)?.as_i64()?).ok()
}

fn text_field(json: &Json, name: &str) -> Option<String> {
    Some(field(json, name)?.as_str()?.to_owned())
}

/// The value of the type `name` that `payload` holds.
fn typed_from(name: &str, payload: &Json) -> Result<Value, JsonError> {
    let bad = || JsonError::BadValue(name.to_owned());
    let int = || payload.as_i64();
    let text = || payload.as_str();
    let number = |number: Option<Number>| number.map(Value::Number).ok_or_else(bad);
    let bound = |name: &str| -> Result<Bound<Value>, JsonError> {
        let Some(bound) = field(payload, name) else {
            return Ok(Bound::Unbounded);
        };
        match (field(bound, "included"), field(bound, "excluded")) {
            (Some(v), None) => Ok(Bound::Included(from_json(v)?)),
            (None, Some(v)) => Ok(Bound::Excluded(from_json(v)?)),
            _ => Err(bad()),
        }
    };
    match name {
        "none" => payload.is_null().then_some(Value::None).ok_or_else(bad),
        "i8" => number(int().and_then(|v| v.try_into().ok()).map(Number::I8)),
        "u8" => number(int().and_then(|v| v.try_into().ok()).map(Number::U8)),
        "i16" => number(int().and_then(|v| v.try_into().ok()).map(Number::I16)),
        "u16" => number(int().and_then(|v| v.try_into().ok()).map(Number::U16)),
        "i32" => number(int().and_then(|v| v.try_into().ok()).map(Number::I32)),
        "u32" => number(int().and_then(|v| v.try_into().ok()).map(Number::U32)),
        "u64" => number(payload.as_u64().map(Number::U64)),
        "float" => number(float_from(payload).map(Number::Float)),
        "f32" => {
            let exact = |v: &f64| v.is_nan() || f64::from(*v as f32) == *v;
            number(float_from(payload).filter(exact).map(|v| Number::F32(v as f32)))
        }
        "f16" => number(float_from(payload).and_then(F16::from_f64).map(Number::F16)),
        "decimal" => number(text().and_then(|v| v.parse().ok()).map(Number::Decimal)),
        "bigdecimal" => number(text().and_then(|v| v.parse().ok()).map(Number::BigDecimal)),
        "bytes" => text().and_then(base64_decode).map(Value::Bytes).ok_or_else(bad),
        "duration" => {
            let secs = field(payload, "secs").and_then(Json::as_u64);
            let nanos = int_field(payload, "nanos").filter(|nanos| *nanos < NANOS_PER_SEC as u32);
            let duration = secs.zip(nanos).map(|(secs, nanos)| Duration { secs, nanos });
            duration.map(Value::Duration).ok_or_else(bad)
        }
        "datetime" => {
            let instant = text().and_then(|v| parse_instant(v.strip_suffix('Z')?));
            instant.map(|(secs, nanos)| Value::Datetime(Datetime { secs, nanos })).ok_or_else(bad)
        }
        "timestamp" => {
            let instant = text().and_then(parse_instant);
            instant.map(|(secs, nanos)| Value::Timestamp(Timestamp { secs, nanos })).ok_or_else(bad)
        }
        "date" => {
            let days = text().and_then(parse_date).and_then(|days| days.try_into().ok());
            days.map(|days| Value::Date(Date { days })).ok_or_else(bad)
        }
        "time" => {
            let nanos = text().and_then(parse_clock);
            nanos.map(|nanos| Value::Time(Time { nanos })).ok_or_else(bad)
        }
        "interval" => {
            let months = int_field(payload, "months").ok_or_else(bad)?;
            let days = int_field(payload, "days").ok_or_else(bad)?;
            let micros = int_field(payload, "micros").ok_or_else(bad)?;
            Ok(Value::Interval(Interval { months, days, micros }))
        }
        "uuid" => text().and_then(|v| v.parse().ok()).map(Value::Uuid).ok_or_else(bad),
        "table" => text().map(|v| Value::Table(v.to_owned())).ok_or_else(bad),
        "regex" => text().map(|v| Value::Regex(v.to_owned())).ok_or_else(bad),
        "record" => {
            let table = text_field(payload, "table").ok_or_else(bad)?;
            let key = from_json(field(payload, "key").ok_or_else(bad)?)?;
            let key = RecordIdKey::try_from(key).map_err(|_| bad())?;
            Ok(Value::RecordId(RecordId { table, key }))
        }
        "file" => {
            let bucket = text_field(payload, "bucket").ok_or_else(bad)?;
            let key = text_field(payload, "key").ok_or_else(bad)?;
            Ok(Value::File(File { bucket, key }))
        }
        "range" if payload.is_object() => {
            Ok(Value::Range(Box::new(Range { start: bound("start")?, end: bound("end")? })))
        }
        "set" => match from_json(payload)? {
            Value::Array(items) if payload.is_array() => Ok(Value::Set(items)),
            _ => Err(bad()),
        },
        "geometry" => geometry_from(payload).map(Value::Geometry).ok_or_else(bad),
        "range" => Err(bad()),
        _ => Err(JsonError::UnknownType(name.to_owned())),
    }
}

fn point_json(point: &Point) -> Json {
    json!([point.x, point.y])
}

fn line_json(points: &[Point]) -> Json {
    Json::Array(points.iter().map(point_json).collect())
}

fn polygon_json(polygon: &Polygon) -> Json {
    let rings = std::iter::once(&polygon.exterior).chain(&polygon.interiors);
    Json::Array(rings.map(|ring| line_json(ring)).collect())
}

/// The geometry as GeoJSON.
fn geometry_json(geometry: &Geometry) -> Json {
    let (name, coordinates) = match geometry {
        Geometry::Point(v) => ("Point", point_json(v)),
        Geometry::Line(v) => ("LineString", line_json(v)),
        Geometry::Polygon(v) => ("Polygon", polygon_json(v)),
        Geometry::MultiPoint(v) => ("MultiPoint", line_json(v)),
        Geometry::MultiLine(v) => {
            ("MultiLineString", Json::Array(v.iter().map(|line| line_json(line)).collect()))
        }
        Geometry::MultiPolygon(v) => {
            ("MultiPolygon", Json::Array(v.iter().map(polygon_json).collect()))
        }
        Geometry::Collection(v) => {
            let geometries: Vec<Json> = v.iter().map(geometry_json).collect();
            return json!({ "type": "GeometryCollection", "geometries": geometries });
        }
    };
    json!({ "type": name, "coordinates": coordinates })
}

fn point_from(json: &Json) -> Option<Point> {
    match json.as_array()?.as_slice() {
        [x, y] => Some(Point { x: x.as_f64()?, y: y.as_f64()? }),
        _ => None,
    }
}

fn line_from(json: &Json) -> Option<Vec<Point>> {
    json.as_array()?.iter().map(point_from).collect()
}

fn lines_from(json: &Json) -> Option<Vec<Vec<Point>>> {
    json.as_array()?.iter().map(line_from).collect()
}

fn polygon_from(json: &Json) -> Option<Polygon> {
    let mut rings = lines_from(json)?.into_iter();
    Some(Polygon { exterior: rings.next()?, interiors: rings.collect() })
}

fn geometry_from(json: &Json) -> Option<Geometry> {
    let coordinates = field(json, "coordinates");
    Some(match field(json, "type")?.as_str()? {
        "Point" => Geometry::Point(point_from(coordinates?)?),
        "LineString" => Geometry::Line(line_from(coordinates?)?),
        "Polygon" => Geometry::Polygon(polygon_from(coordinates?)?),
        "MultiPoint" => Geometry::MultiPoint(line_from(coordinates?)?),
        "MultiLineString" => Geometry::MultiLine(lines_from(coordinates?)?),
        "MultiPolygon" => Geometry::MultiPolygon(
            coordinates?.as_array()?.iter().map(polygon_from).collect::<Option<_>>()?,
        ),
        "GeometryCollection" => {
            let geometries = field(json, "geometries")?.as_array()?;
            Geometry::Collection(geometries.iter().map(geometry_from).collect::<Option<_>>()?)
        }
        _ => return None,
    })
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let byte = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let group = byte(0) << 16 | byte(1) << 8 | byte(2);
        for i in 0..4 {
            let sextet = (group >> (18 - 6 * i) & 63) as usize;
            text.push(if i <= chunk.len() { char::from(BASE64[sextet]) } else { '=' });
        }
    }
    text
}

fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let groups = text.as_bytes().chunks(4);
    let last = groups.len().checked_sub(1);
    let mut bytes = Vec::with_capacity(text.len() / 4 * 3);
    for (i, chunk) in groups.enumerate() {
        let padding = chunk.iter().rev().take_while(|b| **b == b'=').count();
        if chunk.len() != 4 || padding > 2 || (padding > 0 && Some(i) != last) {
            return None;
        }
        let mut group = 0u32;
        for symbol in &chunk[..4 - padding] {
            group = group << 6 | BASE64.iter().position(|b| b == symbol)? as u32;
        }
        group <<= 6 * padding;
        bytes.extend_from_slice(&group.to_be_bytes()[1..4 - padding]);
    }
    Some(bytes)
}

const NANOS_PER_SEC: u64 = 1_000_000_000;
const SECS_PER_DAY: i64 = 86_400;

/// The year, month and day of a count of days since 1970-01-01, in the proleptic Gregorian
/// calendar. The algorithm is Howard Hinnant's `civil_from_days`.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 { shifted_month + 3 } else { shifted_month - 9 };
    (year_of_era + era * 400 + i64::from(month <= 2), month, day)
}

/// The inverse of [`civil_from_days`].
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// `YYYY-MM-DD`, with a sign before a year that is negative or has more than 4 digits.
fn format_date(days: i64) -> String {
    let (year, month, day) = civil_from_days(days);
    let sign = match year {
        ..0 => "-",
        0..=9999 => "",
        _ => "+",
    };
    format!("{sign}{:04}-{month:02}-{day:02}", year.abs())
}

/// The number `text` is, when it is `lengths` digits.
fn digits(text: &str, lengths: std::ops::RangeInclusive<usize>) -> Option<i64> {
    let well_formed = lengths.contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit());
    well_formed.then(|| text.parse().ok())?
}

fn parse_date(text: &str) -> Option<i64> {
    let (sign, unsigned) = match text.as_bytes().first()? {
        b'-' => (-1, &text[1..]),
        b'+' => (1, &text[1..]),
        _ => (1, text),
    };
    let mut parts = unsigned.splitn(3, '-');
    let year = sign * digits(parts.next()?, 4..=12)?;
    let month = digits(parts.next()?, 2..=2)?;
    let day = digits(parts.next()?, 2..=2)?;
    let days = days_from_civil(year, month, day);
    // A day the month does not have comes back as a day of the next month.
    (civil_from_days(days) == (year, month, day)).then_some(days)
}

/// `HH:MM:SS`, with the fraction of the second when there is one.
fn format_clock(nanos: u64) -> String {
    let (secs, fraction) = (nanos / NANOS_PER_SEC, nanos % NANOS_PER_SEC);
    let mut text = format!("{:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60);
    if fraction != 0 {
        text.push('.');
        text.push_str(format!("{fraction:09}").trim_end_matches('0'));
    }
    text
}

/// The nanoseconds since midnight of `HH:MM:SS[.fraction]`, up to `24:00:00`.
fn parse_clock(text: &str) -> Option<u64> {
    let (clock, fraction) = match text.split_once('.') {
        Some((clock, fraction)) => (clock, digits(&format!("{fraction:0<9}"), 9..=9)?),
        None => (text, 0),
    };
    if text.ends_with('.') {
        return None;
    }
    let mut parts = clock.splitn(3, ':');
    let hours = digits(parts.next()?, 2..=2)?;
    let minutes = digits(parts.next()?, 2..=2)?;
    let seconds = digits(parts.next()?, 2..=2)?;
    if minutes > 59 || seconds > 59 {
        return None;
    }
    let nanos = (hours * 3600 + minutes * 60 + seconds) as u64 * NANOS_PER_SEC + fraction as u64;
    (nanos <= Time::MAX_NANOS).then_some(nanos)
}

/// `YYYY-MM-DDTHH:MM:SS[.fraction]` of seconds since 1970-01-01T00:00:00.
fn format_instant(secs: i64, nanos: u32) -> String {
    let clock = secs.rem_euclid(SECS_PER_DAY) as u64 * NANOS_PER_SEC + u64::from(nanos);
    format!("{}T{}", format_date(secs.div_euclid(SECS_PER_DAY)), format_clock(clock))
}

fn parse_instant(text: &str) -> Option<(i64, u32)> {
    let (date, clock) = text.split_once('T')?;
    let nanos = parse_clock(clock).filter(|nanos| *nanos < Time::MAX_NANOS)?;
    let secs = parse_date(date)?.checked_mul(SECS_PER_DAY)?;
    Some((secs.checked_add((nanos / NANOS_PER_SEC) as i64)?, (nanos % NANOS_PER_SEC) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for (bytes, text) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (&[0xDE, 0xAD, 0xBE, 0xEF], "3q2+7w=="),
        ] {
            assert_eq!(base64_encode(bytes), text);
            assert_eq!(base64_decode(text).as_deref(), Some(bytes));
        }
        for text in ["Zg=", "Zg==Zg==", "Z===", "Zm9*"] {
            assert_eq!(base64_decode(text), None, "{text}");
        }
    }

    #[test]
    fn dates() {
        for (days, text) in [
            (0, "1970-01-01"),
            (-1, "1969-12-31"),
            (20_772, "2026-11-15"),
            (11_016, "2000-02-29"),
            (-719_528, "0000-01-01"),
            (-719_529, "-0001-12-31"),
            (2_932_897, "+10000-01-01"),
        ] {
            assert_eq!(format_date(days), text);
            assert_eq!(parse_date(text), Some(days));
        }
        for text in ["2026-02-30", "2026-13-01", "2026-1-01", "26-01-01", "2026-01-01-", ""] {
            assert_eq!(parse_date(text), None, "{text}");
        }
    }

    #[test]
    fn instants() {
        assert_eq!(format_instant(1_794_765_600, 0), "2026-11-15T18:00:00");
        assert_eq!(format_instant(-1, 500_000_000), "1969-12-31T23:59:59.5");
        assert_eq!(parse_instant("1969-12-31T23:59:59.5"), Some((-1, 500_000_000)));
        assert_eq!(parse_instant("2026-11-15T24:00:00"), None);
        assert_eq!(parse_clock("24:00:00"), Some(Time::MAX_NANOS));
        assert_eq!(parse_clock("00:00:00.000000001"), Some(1));
        for text in ["24:00:01", "12:60:00", "12:00", "12:00:00.", "12:00:00.0000000001"] {
            assert_eq!(parse_clock(text), None, "{text}");
        }
    }

    #[test]
    fn typed_values_read_back() {
        let json = json!({
            "name": "Milou",
            "id": { "$uuid": "018f4a2b-7c3d-7e1f-9a0b-1c2d3e4f5a6b" },
            "created": { "$datetime": "2026-11-15T18:00:00Z" },
            "avatar": { "$bytes": "3q2+7w==" },
            "small": { "$u8": 7 },
            "$$price": 1.5,
        });
        let value = from_json(&json).unwrap();
        let Value::Object(fields) = &value else { panic!("{value:?}") };
        assert_eq!(fields["small"], Value::Number(Number::U8(7)));
        assert_eq!(fields["avatar"], Value::Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF]));
        assert_eq!(fields["created"], Value::Datetime(Datetime { secs: 1_794_765_600, nanos: 0 }));
        assert_eq!(fields["$price"], Value::from(1.5));
        assert_eq!(to_json(&value), json);
        assert_eq!(from_json(&json!({ "$nope": 1 })), Err(JsonError::UnknownType("nope".into())));
        assert_eq!(from_json(&json!({ "$u8": 256 })), Err(JsonError::BadValue("u8".into())));
    }
}
