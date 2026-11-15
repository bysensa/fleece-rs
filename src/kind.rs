//! The kinds of SurrealDB, and whether a value of a document is of a kind.
//!
//! [`Kind`] mirrors `surrealdb_types::Kind` variant by variant. [`Kind::conforms`] asks the
//! question of a value where it lies in a document, without decoding it: a store can check
//! what it is given against the kind of a field before it keeps it.
//!
//! The check is of what a value is, with no conversion: `int` takes the 64-bit integer and
//! not an `i8`, `number` takes every numeric kind. Coercing a value to a kind is SurrealDB's
//! own business.

use std::collections::BTreeMap;

use crate::format::{MAX_DEPTH, ext};
use crate::reader::{Item, ValueRef};
use crate::value::{Decimal, Duration};

/// The kind of a value.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Kind {
    /// Every value.
    #[default]
    Any,
    None,
    Null,
    Bool,
    Bytes,
    Datetime,
    Date,
    Time,
    Timestamp,
    Interval,
    Decimal,
    Duration,
    F16,
    F32,
    /// The 64-bit float.
    Float,
    I8,
    I16,
    I32,
    /// The 64-bit integer.
    Int,
    U8,
    U16,
    U32,
    U64,
    /// A number of any kind.
    Number,
    Object,
    String,
    Uuid,
    Regex,
    /// A table, of these names when any are given.
    Table(Vec<String>),
    /// A record, of these tables when any are given.
    Record(Vec<String>),
    /// A geometry, of these shapes when any are given.
    Geometry(Vec<GeometryKind>),
    /// A value of any of these kinds.
    Either(Vec<Kind>),
    /// A set of values of a kind, of at most this many.
    Set(Box<Kind>, Option<u64>),
    /// An array of values of a kind, of at most this many.
    Array(Box<Kind>, Option<u64>),
    /// A function, by the kinds of its arguments and of what it returns. No stored value
    /// is one.
    Function(Option<Vec<Kind>>, Option<Box<Kind>>),
    Range,
    /// One value only.
    Literal(KindLiteral),
    /// A file, of these buckets when any are given.
    File(Vec<String>),
    /// A string that is one of these labels.
    Enum(Vec<String>),
    /// A kind defined elsewhere, by its name; see [`Kind::conforms_with`].
    Named(String),
    BigDecimal,
}

/// The shape of a geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GeometryKind {
    Point,
    Line,
    Polygon,
    MultiPoint,
    MultiLine,
    MultiPolygon,
    Collection,
}

/// A kind that one value is of.
#[derive(Clone, Debug, PartialEq)]
pub enum KindLiteral {
    String(String),
    Integer(i64),
    Float(f64),
    Decimal(Decimal),
    Duration(Duration),
    /// An array of as many items, each of its kind.
    Array(Vec<Kind>),
    /// An object of these fields and no others, each of its kind.
    Object(BTreeMap<String, Kind>),
    Bool(bool),
}

/// The first item of the array that is the payload of an extended value, as a string.
fn first_text<'a>(payload: ValueRef<'a>) -> Option<&'a str> {
    payload.as_array()?.get(0)?.as_str()
}

/// Whether a list of names is empty, which takes every name, or holds `name`.
fn takes(names: &[String], name: Option<&str>) -> bool {
    names.is_empty() || name.is_some_and(|name| names.iter().any(|taken| taken == name))
}

impl Kind {
    /// Whether the value is of this kind. A kind given by name is of no value here.
    pub fn conforms(&self, value: ValueRef<'_>) -> bool {
        self.conforms_with(value, &BTreeMap::new())
    }

    /// Whether the value is of this kind, the kinds given by name being those of `named`.
    pub fn conforms_with(&self, value: ValueRef<'_>, named: &BTreeMap<String, Kind>) -> bool {
        self.check(value, named, 0)
    }

    fn check(&self, value: ValueRef<'_>, named: &BTreeMap<String, Kind>, depth: usize) -> bool {
        // Kinds that name one another in a circle end here.
        if depth > MAX_DEPTH {
            return false;
        }
        let all = |kind: &Kind, items: Item<'_>, most: &Option<u64>| match items {
            Item::Array(array) => {
                most.is_none_or(|most| array.len() as u64 <= most)
                    && array.iter().all(|item| kind.check(item, named, depth + 1))
            }
            _ => false,
        };
        let item = value.item();
        let extended = |kind: u8| matches!(item, Item::Ext(found, _) if found == kind);
        match self {
            Kind::Any => !matches!(item, Item::Invalid | Item::Undefined),
            Kind::None => extended(ext::NONE),
            Kind::Null => matches!(item, Item::Null),
            Kind::Bool => matches!(item, Item::Bool(_)),
            Kind::Bytes => matches!(item, Item::Data(_)),
            Kind::String => matches!(item, Item::String(_)),
            Kind::Object => matches!(item, Item::Dict(_)),
            Kind::Int => matches!(item, Item::Int(_)),
            Kind::Float => matches!(item, Item::Float64(_)),
            Kind::F32 => matches!(item, Item::Float32(_)),
            Kind::Number => match item {
                Item::Int(_) | Item::UInt(_) | Item::Float32(_) | Item::Float64(_) => true,
                Item::Ext(kind, _) => (ext::I8..=ext::BIG_DECIMAL).contains(&kind),
                _ => false,
            },
            Kind::I8 => extended(ext::I8),
            Kind::U8 => extended(ext::U8),
            Kind::I16 => extended(ext::I16),
            Kind::U16 => extended(ext::U16),
            Kind::I32 => extended(ext::I32),
            Kind::U32 => extended(ext::U32),
            Kind::U64 => extended(ext::U64) || matches!(item, Item::UInt(_)),
            Kind::F16 => extended(ext::F16),
            Kind::Decimal => extended(ext::DECIMAL),
            Kind::BigDecimal => extended(ext::BIG_DECIMAL),
            Kind::Duration => extended(ext::DURATION),
            Kind::Datetime => extended(ext::DATETIME),
            Kind::Date => extended(ext::DATE),
            Kind::Time => extended(ext::TIME),
            Kind::Timestamp => extended(ext::TIMESTAMP),
            Kind::Interval => extended(ext::INTERVAL),
            Kind::Uuid => extended(ext::UUID),
            Kind::Regex => extended(ext::REGEX),
            Kind::Range => extended(ext::RANGE),
            Kind::Table(names) => match item {
                Item::Ext(ext::TABLE, Some(payload)) => takes(names, payload.as_str()),
                _ => false,
            },
            Kind::Record(tables) => match item {
                Item::Ext(ext::RECORD_ID, Some(payload)) => takes(tables, first_text(payload)),
                _ => false,
            },
            Kind::File(buckets) => match item {
                Item::Ext(ext::FILE, Some(payload)) => takes(buckets, first_text(payload)),
                _ => false,
            },
            Kind::Geometry(shapes) => match item {
                Item::Ext(ext::GEOMETRY, Some(payload)) => {
                    let shape = payload.as_array().and_then(|array| array.get(0)?.as_i64());
                    shapes.is_empty()
                        || shape.is_some_and(|shape| shapes.iter().any(|s| *s as i64 == shape))
                }
                _ => false,
            },
            Kind::Either(kinds) => kinds.iter().any(|kind| kind.check(value, named, depth + 1)),
            Kind::Array(kind, most) => all(kind, item, most),
            Kind::Set(kind, most) => match item {
                Item::Ext(ext::SET, Some(payload)) => all(kind, payload.item(), most),
                _ => false,
            },
            Kind::Enum(labels) => match item {
                Item::String(text) => labels.iter().any(|label| label == text),
                _ => false,
            },
            Kind::Literal(literal) => literal.check(value, named, depth),
            Kind::Named(name) => {
                named.get(name).is_some_and(|kind| kind.check(value, named, depth + 1))
            }
            Kind::Function(..) => false,
        }
    }
}

impl KindLiteral {
    fn check(&self, value: ValueRef<'_>, named: &BTreeMap<String, Kind>, depth: usize) -> bool {
        match (self, value.item()) {
            (Self::String(literal), Item::String(text)) => literal == text,
            (Self::Integer(literal), Item::Int(number)) => *literal == number,
            (Self::Float(literal), Item::Float64(number)) => *literal == number,
            (Self::Bool(literal), Item::Bool(flag)) => *literal == flag,
            (Self::Decimal(literal), Item::Ext(ext::DECIMAL, Some(payload))) => {
                payload.as_data().is_some_and(|data| data == literal.to_bytes())
            }
            (Self::Duration(literal), Item::Ext(ext::DURATION, Some(payload))) => {
                let mut bytes = [0; 12];
                bytes[..8].copy_from_slice(&literal.secs.to_le_bytes());
                bytes[8..].copy_from_slice(&literal.nanos.to_le_bytes());
                payload.as_data().is_some_and(|data| data == bytes)
            }
            (Self::Array(kinds), Item::Array(array)) => {
                kinds.len() == array.len()
                    && kinds.iter().zip(array.iter()).all(|(k, v)| k.check(v, named, depth + 1))
            }
            (Self::Object(kinds), Item::Dict(dict)) => {
                kinds.len() == dict.len()
                    && kinds.iter().all(|(key, kind)| {
                        dict.get(key).is_some_and(|value| kind.check(value, named, depth + 1))
                    })
            }
            _ => false,
        }
    }
}
