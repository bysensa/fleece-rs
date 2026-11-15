//! [`Value`] to Fleece and back.

use std::ops::Bound;

use crate::encoder::Encoder;
use crate::error::{DecodeError, EncodeError};
use crate::format::{MAX_DEPTH, ext};
use crate::reader::{ArrayRef, DictRef, Doc, Item, ValueRef};
use crate::value::*;
use crate::vector::VectorRef;

/// The value as a document.
pub fn encode(value: &Value) -> Result<Vec<u8>, EncodeError> {
    let mut encoder = Encoder::new();
    encoder.write_value(value)?;
    encoder.finish()
}

/// The root of a document, which is checked first.
pub fn decode(bytes: &[u8]) -> Result<Value, DecodeError> {
    Value::from_ref(Doc::new(bytes)?.root())
}

const GEOMETRY_POINT: i64 = 0;
const GEOMETRY_LINE: i64 = 1;
const GEOMETRY_POLYGON: i64 = 2;
const GEOMETRY_MULTI_POINT: i64 = 3;
const GEOMETRY_MULTI_LINE: i64 = 4;
const GEOMETRY_MULTI_POLYGON: i64 = 5;
const GEOMETRY_COLLECTION: i64 = 6;

const BOUND_UNBOUNDED: i64 = 0;
const BOUND_INCLUDED: i64 = 1;
const BOUND_EXCLUDED: i64 = 2;

/// Seconds, then the nanoseconds of the last one, little-endian.
fn seconds_bytes(secs: [u8; 8], nanos: u32) -> [u8; 12] {
    let mut bytes = [0; 12];
    bytes[..8].copy_from_slice(&secs);
    bytes[8..].copy_from_slice(&nanos.to_le_bytes());
    bytes
}

fn bound_flag<T>(bound: &Bound<T>) -> i64 {
    match bound {
        Bound::Unbounded => BOUND_UNBOUNDED,
        Bound::Included(_) => BOUND_INCLUDED,
        Bound::Excluded(_) => BOUND_EXCLUDED,
    }
}

impl Encoder<'_> {
    /// Writes a value of any type.
    pub fn write_value(&mut self, value: &Value) -> Result<(), EncodeError> {
        match value {
            Value::None => self.write_none(),
            Value::Null => self.write_null(),
            Value::Bool(v) => self.write_bool(*v),
            Value::Number(v) => self.write_number(v),
            Value::String(v) => self.write_str(v),
            Value::Bytes(v) => self.write_data(v),
            Value::Duration(v) => {
                self.begin_ext(ext::DURATION)?;
                self.write_data(&seconds_bytes(v.secs.to_le_bytes(), v.nanos))
            }
            Value::Datetime(v) => {
                self.begin_ext(ext::DATETIME)?;
                self.write_data(&seconds_bytes(v.secs.to_le_bytes(), v.nanos))
            }
            Value::Timestamp(v) => {
                self.begin_ext(ext::TIMESTAMP)?;
                self.write_data(&seconds_bytes(v.secs.to_le_bytes(), v.nanos))
            }
            Value::Uuid(v) => {
                self.begin_ext(ext::UUID)?;
                self.write_data(&v.0)
            }
            Value::Date(v) => {
                self.begin_ext(ext::DATE)?;
                self.write_int(i64::from(v.days))
            }
            Value::Time(v) => {
                self.begin_ext(ext::TIME)?;
                self.write_uint(v.nanos)
            }
            Value::Interval(v) => {
                let mut bytes = [0; 16];
                bytes[..4].copy_from_slice(&v.months.to_le_bytes());
                bytes[4..8].copy_from_slice(&v.days.to_le_bytes());
                bytes[8..].copy_from_slice(&v.micros.to_le_bytes());
                self.begin_ext(ext::INTERVAL)?;
                self.write_data(&bytes)
            }
            Value::Table(v) => {
                self.begin_ext(ext::TABLE)?;
                self.write_str(v)
            }
            Value::Regex(v) => {
                self.begin_ext(ext::REGEX)?;
                self.write_str(v)
            }
            Value::RecordId(v) => {
                self.begin_ext(ext::RECORD_ID)?;
                self.begin_array()?;
                self.write_str(&v.table)?;
                self.write_record_key(&v.key)?;
                self.end_array()
            }
            Value::File(v) => {
                self.begin_ext(ext::FILE)?;
                self.begin_array()?;
                self.write_str(&v.bucket)?;
                self.write_str(&v.key)?;
                self.end_array()
            }
            Value::Range(v) => {
                self.write_range(&v.start, &v.end, |encoder, v| encoder.write_value(v))
            }
            Value::Array(items) => self.write_array(items),
            Value::Set(items) => {
                self.begin_ext(ext::SET)?;
                self.write_array(items)
            }
            Value::Object(fields) => self.write_object(fields),
            Value::Geometry(v) => self.write_geometry(v),
            Value::Vector(v) => {
                self.begin_ext(ext::VECTOR)?;
                self.write_data_from(v.byte_len(), |out| v.write_bytes(out))
            }
        }
    }

    fn write_number(&mut self, number: &Number) -> Result<(), EncodeError> {
        let (kind, value) = match number {
            Number::Int(v) => return self.write_int(*v),
            Number::Float(v) => return self.write_f64(*v),
            Number::F32(v) => return self.write_f32(*v),
            Number::Decimal(v) => {
                self.begin_ext(ext::DECIMAL)?;
                return self.write_data(&v.to_bytes());
            }
            Number::BigDecimal(v) => {
                self.begin_ext(ext::BIG_DECIMAL)?;
                self.begin_array()?;
                self.write_uint(u64::from(v.scale()))?;
                self.write_data(v.coefficient())?;
                return self.end_array();
            }
            Number::U64(v) => {
                self.begin_ext(ext::U64)?;
                return self.write_uint(*v);
            }
            Number::I8(v) => (ext::I8, i64::from(*v)),
            Number::U8(v) => (ext::U8, i64::from(*v)),
            Number::I16(v) => (ext::I16, i64::from(*v)),
            Number::U16(v) => (ext::U16, i64::from(*v)),
            Number::I32(v) => (ext::I32, i64::from(*v)),
            Number::U32(v) => (ext::U32, i64::from(*v)),
            Number::F16(v) => (ext::F16, i64::from(v.0)),
        };
        self.begin_ext(kind)?;
        self.write_int(value)
    }

    fn write_array(&mut self, items: &[Value]) -> Result<(), EncodeError> {
        self.begin_array()?;
        for item in items {
            self.write_value(item)?;
        }
        self.end_array()
    }

    fn write_object(&mut self, fields: &Object) -> Result<(), EncodeError> {
        self.begin_dict()?;
        for (key, value) in fields {
            self.write_key(key)?;
            self.write_value(value)?;
        }
        self.end_dict()
    }

    /// A range as `[flags, start, end]`: the flags hold the kind of the start bound in their
    /// low 2 bits and that of the end bound in the next 2, and a missing bound is null.
    fn write_range<T>(
        &mut self,
        start: &Bound<T>,
        end: &Bound<T>,
        write: impl Fn(&mut Self, &T) -> Result<(), EncodeError>,
    ) -> Result<(), EncodeError> {
        self.begin_ext(ext::RANGE)?;
        self.begin_array()?;
        self.write_int(bound_flag(start) | bound_flag(end) << 2)?;
        for bound in [start, end] {
            match bound {
                Bound::Included(v) | Bound::Excluded(v) => write(self, v)?,
                Bound::Unbounded => self.write_null()?,
            }
        }
        self.end_array()
    }

    fn write_record_key(&mut self, key: &RecordIdKey) -> Result<(), EncodeError> {
        match key {
            RecordIdKey::Number(v) => self.write_int(*v),
            RecordIdKey::String(v) => self.write_str(v),
            RecordIdKey::Uuid(v) => {
                self.begin_ext(ext::UUID)?;
                self.write_data(&v.0)
            }
            RecordIdKey::Array(items) => self.write_array(items),
            RecordIdKey::Object(fields) => self.write_object(fields),
            RecordIdKey::Range(range) => {
                self.write_range(&range.start, &range.end, Self::write_record_key)
            }
        }
    }

    /// The points as pairs of little-endian doubles.
    fn write_points(&mut self, points: &[Point]) -> Result<(), EncodeError> {
        self.write_data_from(points.len() * 16, |out| {
            for point in points {
                out.extend_from_slice(&point.x.to_le_bytes());
                out.extend_from_slice(&point.y.to_le_bytes());
            }
        })
    }

    fn write_rings(&mut self, rings: &[Vec<Point>]) -> Result<(), EncodeError> {
        self.begin_array()?;
        for ring in rings {
            self.write_points(ring)?;
        }
        self.end_array()
    }

    /// A polygon as an array of rings, the outer one first.
    fn write_polygon(&mut self, polygon: &Polygon) -> Result<(), EncodeError> {
        self.begin_array()?;
        self.write_points(&polygon.exterior)?;
        for ring in &polygon.interiors {
            self.write_points(ring)?;
        }
        self.end_array()
    }

    fn write_geometry(&mut self, geometry: &Geometry) -> Result<(), EncodeError> {
        self.begin_ext(ext::GEOMETRY)?;
        self.begin_array()?;
        match geometry {
            Geometry::Point(point) => {
                self.write_int(GEOMETRY_POINT)?;
                self.write_points(std::slice::from_ref(point))?;
            }
            Geometry::Line(points) => {
                self.write_int(GEOMETRY_LINE)?;
                self.write_points(points)?;
            }
            Geometry::Polygon(polygon) => {
                self.write_int(GEOMETRY_POLYGON)?;
                self.write_polygon(polygon)?;
            }
            Geometry::MultiPoint(points) => {
                self.write_int(GEOMETRY_MULTI_POINT)?;
                self.write_points(points)?;
            }
            Geometry::MultiLine(lines) => {
                self.write_int(GEOMETRY_MULTI_LINE)?;
                self.write_rings(lines)?;
            }
            Geometry::MultiPolygon(polygons) => {
                self.write_int(GEOMETRY_MULTI_POLYGON)?;
                self.begin_array()?;
                for polygon in polygons {
                    self.write_polygon(polygon)?;
                }
                self.end_array()?;
            }
            Geometry::Collection(geometries) => {
                self.write_int(GEOMETRY_COLLECTION)?;
                self.begin_array()?;
                for geometry in geometries {
                    self.write_geometry(geometry)?;
                }
                self.end_array()?;
            }
        }
        self.end_array()
    }
}

/// Reads the payload of one extended value, every mismatch being the same error.
struct Payload {
    error: DecodeError,
    depth: usize,
}

impl Payload {
    fn int(&self, value: ValueRef<'_>) -> Result<i64, DecodeError> {
        value.as_i64().ok_or(self.error.clone())
    }

    fn sized<T: TryFrom<i64>>(&self, value: ValueRef<'_>) -> Result<T, DecodeError> {
        T::try_from(self.int(value)?).map_err(|_| self.error.clone())
    }

    fn data<const N: usize>(&self, value: ValueRef<'_>) -> Result<[u8; N], DecodeError> {
        value.as_data().and_then(|data| data.try_into().ok()).ok_or(self.error.clone())
    }

    fn string(&self, value: ValueRef<'_>) -> Result<String, DecodeError> {
        value.as_str().map(str::to_owned).ok_or(self.error.clone())
    }

    /// The items of an array of exactly `N` items.
    fn items<'a, const N: usize>(
        &self,
        value: ValueRef<'a>,
    ) -> Result<[ValueRef<'a>; N], DecodeError> {
        let array = value.as_array().filter(|array| array.len() == N);
        let array = array.ok_or(self.error.clone())?;
        let mut items = [value; N];
        for (index, item) in items.iter_mut().enumerate() {
            *item = array.get(index).ok_or(self.error.clone())?;
        }
        Ok(items)
    }

    /// The items of an array of any length.
    fn list<'a>(
        &self,
        value: ValueRef<'a>,
    ) -> Result<impl Iterator<Item = ValueRef<'a>> + use<'a>, DecodeError> {
        Ok(value.as_array().ok_or(self.error.clone())?.iter())
    }

    /// Seconds and nanoseconds, the nanoseconds being less than a second.
    fn seconds(&self, value: ValueRef<'_>) -> Result<([u8; 8], u32), DecodeError> {
        let bytes: [u8; 12] = self.data(value)?;
        let (secs, nanos) = bytes.split_at(8);
        let nanos = u32::from_le_bytes(nanos.try_into().map_err(|_| self.error.clone())?);
        let secs = secs.try_into().map_err(|_| self.error.clone())?;
        if nanos < 1_000_000_000 { Ok((secs, nanos)) } else { Err(self.error.clone()) }
    }

    fn points(&self, value: ValueRef<'_>) -> Result<Vec<Point>, DecodeError> {
        let (pairs, rest) = value.as_data().ok_or(self.error.clone())?.as_chunks::<16>();
        if !rest.is_empty() {
            return Err(self.error.clone());
        }
        let double = |bytes: &[u8]| bytes.try_into().map(f64::from_le_bytes).unwrap_or_default();
        Ok(pairs
            .iter()
            .map(|pair| Point { x: double(&pair[..8]), y: double(&pair[8..]) })
            .collect())
    }

    fn rings(&self, value: ValueRef<'_>) -> Result<Vec<Vec<Point>>, DecodeError> {
        self.list(value)?.map(|ring| self.points(ring)).collect()
    }

    fn polygon(&self, value: ValueRef<'_>) -> Result<Polygon, DecodeError> {
        let mut rings = self.rings(value)?.into_iter();
        let exterior = rings.next().ok_or(self.error.clone())?;
        Ok(Polygon { exterior, interiors: rings.collect() })
    }

    fn geometry(&self, value: ValueRef<'_>) -> Result<Geometry, DecodeError> {
        let [kind, shape] = self.items(value)?;
        Ok(match self.int(kind)? {
            GEOMETRY_POINT => match self.points(shape)?[..] {
                [point] => Geometry::Point(point),
                _ => return Err(self.error.clone()),
            },
            GEOMETRY_LINE => Geometry::Line(self.points(shape)?),
            GEOMETRY_POLYGON => Geometry::Polygon(self.polygon(shape)?),
            GEOMETRY_MULTI_POINT => Geometry::MultiPoint(self.points(shape)?),
            GEOMETRY_MULTI_LINE => Geometry::MultiLine(self.rings(shape)?),
            GEOMETRY_MULTI_POLYGON => Geometry::MultiPolygon(
                self.list(shape)?.map(|polygon| self.polygon(polygon)).collect::<Result<_, _>>()?,
            ),
            GEOMETRY_COLLECTION => {
                if self.depth >= MAX_DEPTH {
                    return Err(DecodeError::TooDeep);
                }
                let inner = Payload { error: self.error.clone(), depth: self.depth + 2 };
                let geometries = self.list(shape)?.map(|geometry| match geometry.item() {
                    Item::Ext(ext::GEOMETRY, Some(payload)) => inner.geometry(payload),
                    _ => Err(self.error.clone()),
                });
                Geometry::Collection(geometries.collect::<Result<_, _>>()?)
            }
            _ => return Err(self.error.clone()),
        })
    }

    fn range(&self, value: ValueRef<'_>) -> Result<Range, DecodeError> {
        let [flags, start, end] = self.items(value)?;
        let flags = self.int(flags)?;
        let bound = |flag: i64, value: ValueRef<'_>| {
            Ok(match flag {
                BOUND_UNBOUNDED if value.is_null() => Bound::Unbounded,
                BOUND_INCLUDED => Bound::Included(decode_value(value, self.depth + 1)?),
                BOUND_EXCLUDED => Bound::Excluded(decode_value(value, self.depth + 1)?),
                _ => return Err(self.error.clone()),
            })
        };
        if !(0..16).contains(&flags) {
            return Err(self.error.clone());
        }
        Ok(Range { start: bound(flags & 3, start)?, end: bound(flags >> 2, end)? })
    }

    fn record(&self, payload: ValueRef<'_>) -> Result<Value, DecodeError> {
        let [table, key] = self.items(payload)?;
        let key = decode_value(key, self.depth + 1)?;
        let key = RecordIdKey::try_from(key).map_err(|_| self.error.clone())?;
        Ok(Value::RecordId(RecordId { table: self.string(table)?, key }))
    }

    /// An extended value that holds no other value. The kinds that do are read by
    /// [`decode_extended`], apart from this function, so that what is called once for
    /// every level of a deep document stays small on the stack.
    fn leaf(&self, kind: u8, payload: ValueRef<'_>) -> Result<Value, DecodeError> {
        let number = |number| Ok(Value::Number(number));
        match kind {
            ext::I8 => number(Number::I8(self.sized(payload)?)),
            ext::U8 => number(Number::U8(self.sized(payload)?)),
            ext::I16 => number(Number::I16(self.sized(payload)?)),
            ext::U16 => number(Number::U16(self.sized(payload)?)),
            ext::I32 => number(Number::I32(self.sized(payload)?)),
            ext::U32 => number(Number::U32(self.sized(payload)?)),
            ext::U64 => number(Number::U64(payload.as_u64().ok_or(self.error.clone())?)),
            ext::F16 => number(Number::F16(F16(self.sized(payload)?))),
            ext::DECIMAL => {
                let decimal = Decimal::from_bytes(self.data(payload)?);
                number(Number::Decimal(decimal.ok_or(self.error.clone())?))
            }
            ext::BIG_DECIMAL => {
                let [scale, coefficient] = self.items(payload)?;
                let coefficient = coefficient.as_data().ok_or(self.error.clone())?;
                let decimal = BigDecimal::new(coefficient, self.sized(scale)?);
                number(Number::BigDecimal(decimal.ok_or(self.error.clone())?))
            }
            ext::DURATION => {
                let (secs, nanos) = self.seconds(payload)?;
                Ok(Value::Duration(Duration { secs: u64::from_le_bytes(secs), nanos }))
            }
            ext::DATETIME => {
                let (secs, nanos) = self.seconds(payload)?;
                Ok(Value::Datetime(Datetime { secs: i64::from_le_bytes(secs), nanos }))
            }
            ext::TIMESTAMP => {
                let (secs, nanos) = self.seconds(payload)?;
                Ok(Value::Timestamp(Timestamp { secs: i64::from_le_bytes(secs), nanos }))
            }
            ext::UUID => Ok(Value::Uuid(Uuid(self.data(payload)?))),
            ext::DATE => Ok(Value::Date(Date { days: self.sized(payload)? })),
            ext::TIME => {
                let nanos = payload.as_u64().filter(|nanos| *nanos <= Time::MAX_NANOS);
                Ok(Value::Time(Time { nanos: nanos.ok_or(self.error.clone())? }))
            }
            ext::INTERVAL => {
                let bytes: [u8; 16] = self.data(payload)?;
                let part = |range: std::ops::Range<usize>| {
                    let mut wide = [0; 8];
                    wide[..range.len()].copy_from_slice(&bytes[range]);
                    wide
                };
                Ok(Value::Interval(Interval {
                    months: i64::from_le_bytes(part(0..4)) as i32,
                    days: i64::from_le_bytes(part(4..8)) as i32,
                    micros: i64::from_le_bytes(part(8..16)),
                }))
            }
            ext::TABLE => Ok(Value::Table(self.string(payload)?)),
            ext::REGEX => Ok(Value::Regex(self.string(payload)?)),
            ext::FILE => {
                let [bucket, key] = self.items(payload)?;
                Ok(Value::File(File { bucket: self.string(bucket)?, key: self.string(key)? }))
            }
            ext::VECTOR => {
                let vector = payload.as_data().and_then(VectorRef::parse);
                Ok(Value::Vector(vector.ok_or(self.error.clone())?.to_vector()))
            }
            _ => Err(self.error.clone()),
        }
    }
}

fn decode_value(value: ValueRef<'_>, depth: usize) -> Result<Value, DecodeError> {
    let pos = value.position();
    // Ranges and record keys nest without a collection of their own in between.
    if depth > MAX_DEPTH {
        return Err(DecodeError::TooDeep);
    }
    Ok(match value.item() {
        Item::Null => Value::Null,
        Item::Bool(v) => Value::Bool(v),
        Item::Int(v) => Value::Number(Number::Int(v)),
        Item::UInt(v) => Value::Number(Number::U64(v)),
        Item::Float32(v) => Value::Number(Number::F32(v)),
        Item::Float64(v) => Value::Number(Number::Float(v)),
        Item::String(v) => Value::String(v.to_owned()),
        Item::Data(v) => Value::Bytes(v.to_vec()),
        Item::Array(array) => Value::Array(decode_items(array, depth)?),
        Item::Dict(dict) => Value::Object(decode_fields(dict, depth)?),
        Item::Ext(ext::NONE, _) => Value::None,
        Item::Ext(kind, Some(payload)) => return decode_extended(kind, payload, pos, depth),
        Item::Ext(..) | Item::Invalid => return Err(DecodeError::Malformed { pos }),
        Item::Undefined => return Err(DecodeError::Undefined { pos }),
    })
}

/// The items of an array, straight into a vector of their number: collecting through a
/// `Result` would first gather them in a vector that grows as it goes.
fn decode_items(array: ArrayRef<'_>, depth: usize) -> Result<Vec<Value>, DecodeError> {
    if depth >= MAX_DEPTH {
        return Err(DecodeError::TooDeep);
    }
    let mut items = Vec::with_capacity(array.len());
    for item in array.iter() {
        items.push(decode_value(item, depth + 1)?);
    }
    Ok(items)
}

fn decode_fields(dict: DictRef<'_>, depth: usize) -> Result<Object, DecodeError> {
    if depth >= MAX_DEPTH {
        return Err(DecodeError::TooDeep);
    }
    let mut fields = Object::new();
    for (key, value) in dict.iter() {
        fields.insert(key.to_owned(), decode_value(value, depth + 1)?);
    }
    Ok(fields)
}

/// An extended value. The kinds that hold other values are told apart here and each read
/// by a function of its own; all the others, which end the descent, by [`Payload::leaf`].
fn decode_extended(
    kind: u8,
    payload: ValueRef<'_>,
    pos: usize,
    depth: usize,
) -> Result<Value, DecodeError> {
    let read = Payload { error: DecodeError::BadPayload { pos, kind }, depth };
    match kind {
        ext::SET => match payload.as_array() {
            Some(array) => Ok(Value::Set(decode_items(array, depth)?)),
            None => Err(read.error),
        },
        ext::RANGE => Ok(Value::Range(Box::new(read.range(payload)?))),
        ext::RECORD_ID => read.record(payload),
        ext::GEOMETRY => Ok(Value::Geometry(read.geometry(payload)?)),
        _ => read.leaf(kind, payload),
    }
}

impl Value {
    /// The owned copy of a value of a document.
    pub fn from_ref(value: ValueRef<'_>) -> Result<Value, DecodeError> {
        decode_value(value, 0)
    }
}
