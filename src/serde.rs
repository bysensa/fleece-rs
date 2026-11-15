//! The types of an application to documents and back, through `serde`, with no [`Value`]
//! in between.
//!
//! Reading lends out of the document: a `&str` or `&[u8]` field of a struct points into the
//! bytes it was read from, and nothing is allocated for it.
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
//! struct User<'a> {
//!     name: &'a str,
//!     age: u8,
//!     id: fleece::Uuid,
//! }
//!
//! let user = User { name: "Milou", age: 9, id: fleece::Uuid([7; 16]) };
//! let bytes = fleece::serde::to_vec(&user)?;
//! assert_eq!(fleece::serde::from_slice::<User>(&bytes)?, user);
//! # Ok(())
//! # }
//! ```
//!
//! # What is written
//!
//! A struct and a map are a dictionary, a sequence and a tuple an array, an enum variant
//! without data its name, one with data a dictionary of the name and the data. `None` and
//! `()` are null. Integers of every width are the integer of standard Fleece, and floats
//! keep their width: the sized numbers of SurrealDB are written only for a field that is a
//! [`Number`]. A field whose type is one of this crate ([`Value`], [`Uuid`], [`Datetime`],
//! [`Decimal`] and the others) is written as the extended value it is.
//!
//! Those types go through other serializers too, as the bytes of a small document, which
//! serves formats that keep bytes; for JSON use [`crate::json`].

use std::fmt::{self, Display, Formatter};

use ::serde::de::value::{BorrowedStrDeserializer, StrDeserializer};
use ::serde::de::{self, DeserializeSeed, IntoDeserializer, Visitor};
use ::serde::ser::{self, Impossible};
use ::serde::{Deserialize, Deserializer as _, Serialize};

use crate::codec::{decode, encode};
use crate::encoder::Encoder;
use crate::error::{DecodeError, EncodeError};
use crate::format::ext;
use crate::reader::{ArrayRef, DictIter, Doc, Item, ValueRef};
use crate::value::*;

/// The name of the newtype a value of this crate passes through `serde` as: its contents
/// are the bytes of a document that holds the value.
const TOKEN: &str = "$fleece::Value";

/// Why a type could not be written or read.
#[derive(Clone, Debug, PartialEq)]
pub enum Error {
    /// What the type, or this module, has to say.
    Message(String),
    Encode(EncodeError),
    Decode(DecodeError),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Message(message) => f.write_str(message),
            Self::Encode(e) => write!(f, "{e}"),
            Self::Decode(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

impl ser::Error for Error {
    fn custom<T: Display>(message: T) -> Self {
        Self::Message(message.to_string())
    }
}

impl de::Error for Error {
    fn custom<T: Display>(message: T) -> Self {
        Self::Message(message.to_string())
    }
}

impl From<EncodeError> for Error {
    fn from(e: EncodeError) -> Self {
        Self::Encode(e)
    }
}

impl From<DecodeError> for Error {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

/// The document of a value of any type that `serde` writes.
pub fn to_vec<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>, Error> {
    let mut encoder = Encoder::new();
    to_encoder(value, &mut encoder)?;
    Ok(encoder.finish()?)
}

/// Writes a value into an encoder: one with shared keys, or as a part of a larger document.
pub fn to_encoder<T: Serialize + ?Sized>(
    value: &T,
    encoder: &mut Encoder<'_>,
) -> Result<(), Error> {
    value.serialize(Writer { encoder })
}

/// The root of a document, which is checked first, as a value of a type `serde` reads.
pub fn from_slice<'de, T: Deserialize<'de>>(bytes: &'de [u8]) -> Result<T, Error> {
    from_ref(Doc::new(bytes)?.root())
}

/// A value of a document as a value of a type `serde` reads.
pub fn from_ref<'de, T: Deserialize<'de>>(value: ValueRef<'de>) -> Result<T, Error> {
    T::deserialize(Reader { value })
}

fn refused<T>(what: &str) -> Result<T, Error> {
    Err(Error::Message(what.to_owned()))
}

struct Writer<'e, 'k> {
    encoder: &'e mut Encoder<'k>,
}

/// A sequence, a map or a struct being written, and how many dictionaries end with it:
/// a variant with data lies in a dictionary of its own.
struct Compound<'e, 'k> {
    encoder: &'e mut Encoder<'k>,
    dict: bool,
    in_variant: bool,
}

impl<'e, 'k> Writer<'e, 'k> {
    fn compound(self, dict: bool, variant: Option<&str>) -> Result<Compound<'e, 'k>, Error> {
        if let Some(variant) = variant {
            self.encoder.begin_dict()?;
            self.encoder.write_key(variant)?;
        }
        if dict {
            self.encoder.begin_dict()?;
        } else {
            self.encoder.begin_array()?;
        }
        Ok(Compound { encoder: self.encoder, dict, in_variant: variant.is_some() })
    }
}

impl Compound<'_, '_> {
    fn item<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        value.serialize(Writer { encoder: &mut *self.encoder })
    }

    fn field<T: Serialize + ?Sized>(&mut self, key: &str, value: &T) -> Result<(), Error> {
        self.encoder.write_key(key)?;
        self.item(value)
    }

    fn finish(self) -> Result<(), Error> {
        if self.dict {
            self.encoder.end_dict()?;
        } else {
            self.encoder.end_array()?;
        }
        if self.in_variant {
            self.encoder.end_dict()?;
        }
        Ok(())
    }
}

impl<'e, 'k> ser::Serializer for Writer<'e, 'k> {
    type Ok = ();
    type Error = Error;
    type SerializeSeq = Compound<'e, 'k>;
    type SerializeTuple = Compound<'e, 'k>;
    type SerializeTupleStruct = Compound<'e, 'k>;
    type SerializeTupleVariant = Compound<'e, 'k>;
    type SerializeMap = Compound<'e, 'k>;
    type SerializeStruct = Compound<'e, 'k>;
    type SerializeStructVariant = Compound<'e, 'k>;

    fn serialize_bool(self, v: bool) -> Result<(), Error> {
        Ok(self.encoder.write_bool(v)?)
    }

    fn serialize_i8(self, v: i8) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i16(self, v: i16) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i32(self, v: i32) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_i64(self, v: i64) -> Result<(), Error> {
        Ok(self.encoder.write_int(v)?)
    }

    fn serialize_i128(self, v: i128) -> Result<(), Error> {
        match i64::try_from(v) {
            Ok(v) => self.serialize_i64(v),
            Err(_) => refused("an integer of more than 64 bits"),
        }
    }

    fn serialize_u8(self, v: u8) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_u16(self, v: u16) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_u32(self, v: u32) -> Result<(), Error> {
        self.serialize_i64(i64::from(v))
    }

    fn serialize_u64(self, v: u64) -> Result<(), Error> {
        match i64::try_from(v) {
            Ok(v) => self.serialize_i64(v),
            Err(_) => Ok(self.encoder.write_uint(v)?),
        }
    }

    fn serialize_u128(self, v: u128) -> Result<(), Error> {
        match u64::try_from(v) {
            Ok(v) => self.serialize_u64(v),
            Err(_) => refused("an integer of more than 64 bits"),
        }
    }

    fn serialize_f32(self, v: f32) -> Result<(), Error> {
        Ok(self.encoder.write_f32(v)?)
    }

    fn serialize_f64(self, v: f64) -> Result<(), Error> {
        Ok(self.encoder.write_f64(v)?)
    }

    fn serialize_char(self, v: char) -> Result<(), Error> {
        self.serialize_str(v.encode_utf8(&mut [0; 4]))
    }

    fn serialize_str(self, v: &str) -> Result<(), Error> {
        Ok(self.encoder.write_str(v)?)
    }

    fn serialize_bytes(self, v: &[u8]) -> Result<(), Error> {
        Ok(self.encoder.write_data(v)?)
    }

    fn serialize_none(self) -> Result<(), Error> {
        self.serialize_unit()
    }

    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<(), Error> {
        value.serialize(self)
    }

    fn serialize_unit(self) -> Result<(), Error> {
        Ok(self.encoder.write_null()?)
    }

    fn serialize_unit_struct(self, _: &'static str) -> Result<(), Error> {
        self.serialize_unit()
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<(), Error> {
        self.serialize_str(variant)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        name: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        if name == TOKEN {
            // A value of this crate: what it holds is a document, whose root is copied.
            return value.serialize(Copier { encoder: self.encoder });
        }
        value.serialize(self)
    }

    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.encoder.begin_dict()?;
        self.encoder.write_key(variant)?;
        value.serialize(Writer { encoder: &mut *self.encoder })?;
        Ok(self.encoder.end_dict()?)
    }

    fn serialize_seq(self, _: Option<usize>) -> Result<Compound<'e, 'k>, Error> {
        self.compound(false, None)
    }

    fn serialize_tuple(self, _: usize) -> Result<Compound<'e, 'k>, Error> {
        self.compound(false, None)
    }

    fn serialize_tuple_struct(self, _: &'static str, _: usize) -> Result<Compound<'e, 'k>, Error> {
        self.compound(false, None)
    }

    fn serialize_tuple_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Compound<'e, 'k>, Error> {
        self.compound(false, Some(variant))
    }

    fn serialize_map(self, _: Option<usize>) -> Result<Compound<'e, 'k>, Error> {
        self.compound(true, None)
    }

    fn serialize_struct(self, _: &'static str, _: usize) -> Result<Compound<'e, 'k>, Error> {
        self.compound(true, None)
    }

    fn serialize_struct_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
        _: usize,
    ) -> Result<Compound<'e, 'k>, Error> {
        self.compound(true, Some(variant))
    }
}

impl ser::SerializeSeq for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.item(value)
    }

    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

impl ser::SerializeTuple for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;

    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.item(value)
    }

    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

impl ser::SerializeTupleStruct for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.item(value)
    }

    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

impl ser::SerializeTupleVariant for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.item(value)
    }

    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

impl ser::SerializeMap for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;

    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), Error> {
        let key = key.serialize(KeyWriter)?;
        Ok(self.encoder.write_key(&key)?)
    }

    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), Error> {
        self.item(value)
    }

    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

impl ser::SerializeStruct for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.field(key, value)
    }

    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

impl ser::SerializeStructVariant for Compound<'_, '_> {
    type Ok = ();
    type Error = Error;

    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        self.field(key, value)
    }

    fn end(self) -> Result<(), Error> {
        self.finish()
    }
}

/// The methods of a serializer that takes only a few kinds of value: every one named here
/// refuses with the reason given.
macro_rules! refuse {
    ($why:literal: $($method:ident($($argument:ty),*) -> $result:ty,)*) => {
        $(fn $method(self, $(_: $argument),*) -> Result<$result, Error> {
            refused($why)
        })*

        fn serialize_some<T: Serialize + ?Sized>(self, _: &T) -> Result<Self::Ok, Error> {
            refused($why)
        }

        fn serialize_newtype_variant<T: Serialize + ?Sized>(
            self,
            _: &'static str,
            _: u32,
            _: &'static str,
            _: &T,
        ) -> Result<Self::Ok, Error> {
            refused($why)
        }
    };
}

/// The associated types of a serializer that writes no collection.
macro_rules! no_collections {
    () => {
        type Error = Error;
        type SerializeSeq = Impossible<Self::Ok, Error>;
        type SerializeTuple = Impossible<Self::Ok, Error>;
        type SerializeTupleStruct = Impossible<Self::Ok, Error>;
        type SerializeTupleVariant = Impossible<Self::Ok, Error>;
        type SerializeMap = Impossible<Self::Ok, Error>;
        type SerializeStruct = Impossible<Self::Ok, Error>;
        type SerializeStructVariant = Impossible<Self::Ok, Error>;
    };
}

/// Turns the key of a map into the string a dictionary takes.
struct KeyWriter;

impl ser::Serializer for KeyWriter {
    type Ok = String;
    no_collections!();

    fn serialize_str(self, v: &str) -> Result<String, Error> {
        Ok(v.to_owned())
    }

    fn serialize_char(self, v: char) -> Result<String, Error> {
        Ok(v.to_string())
    }

    fn serialize_i8(self, v: i8) -> Result<String, Error> {
        Ok(v.to_string())
    }

    fn serialize_i16(self, v: i16) -> Result<String, Error> {
        Ok(v.to_string())
    }

    fn serialize_i32(self, v: i32) -> Result<String, Error> {
        Ok(v.to_string())
    }

    fn serialize_i64(self, v: i64) -> Result<String, Error> {
        Ok(v.to_string())
    }

    fn serialize_u8(self, v: u8) -> Result<String, Error> {
        Ok(v.to_string())
    }

    fn serialize_u16(self, v: u16) -> Result<String, Error> {
        Ok(v.to_string())
    }

    fn serialize_u32(self, v: u32) -> Result<String, Error> {
        Ok(v.to_string())
    }

    fn serialize_u64(self, v: u64) -> Result<String, Error> {
        Ok(v.to_string())
    }

    fn serialize_unit_variant(
        self,
        _: &'static str,
        _: u32,
        variant: &'static str,
    ) -> Result<String, Error> {
        Ok(variant.to_owned())
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<String, Error> {
        value.serialize(self)
    }

    refuse! { "the key of a map has to be a string or an integer":
        serialize_bool(bool) -> String,
        serialize_f32(f32) -> String,
        serialize_f64(f64) -> String,
        serialize_bytes(&[u8]) -> String,
        serialize_none() -> String,
        serialize_unit() -> String,
        serialize_unit_struct(&'static str) -> String,
        serialize_seq(Option<usize>) -> Self::SerializeSeq,
        serialize_tuple(usize) -> Self::SerializeTuple,
        serialize_tuple_struct(&'static str, usize) -> Self::SerializeTupleStruct,
        serialize_tuple_variant(&'static str, u32, &'static str, usize)
            -> Self::SerializeTupleVariant,
        serialize_map(Option<usize>) -> Self::SerializeMap,
        serialize_struct(&'static str, usize) -> Self::SerializeStruct,
        serialize_struct_variant(&'static str, u32, &'static str, usize)
            -> Self::SerializeStructVariant,
    }
}

/// Takes the bytes of a document and copies its root into the encoder.
struct Copier<'e, 'k> {
    encoder: &'e mut Encoder<'k>,
}

impl ser::Serializer for Copier<'_, '_> {
    type Ok = ();
    no_collections!();

    fn serialize_bytes(self, document: &[u8]) -> Result<(), Error> {
        Ok(self.encoder.write_copy(Doc::trusted(document).root())?)
    }

    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _: &'static str,
        value: &T,
    ) -> Result<(), Error> {
        value.serialize(self)
    }

    refuse! { "a value of this crate passes as the bytes of a document":
        serialize_bool(bool) -> (),
        serialize_i8(i8) -> (),
        serialize_i16(i16) -> (),
        serialize_i32(i32) -> (),
        serialize_i64(i64) -> (),
        serialize_u8(u8) -> (),
        serialize_u16(u16) -> (),
        serialize_u32(u32) -> (),
        serialize_u64(u64) -> (),
        serialize_f32(f32) -> (),
        serialize_f64(f64) -> (),
        serialize_char(char) -> (),
        serialize_str(&str) -> (),
        serialize_none() -> (),
        serialize_unit() -> (),
        serialize_unit_struct(&'static str) -> (),
        serialize_unit_variant(&'static str, u32, &'static str) -> (),
        serialize_seq(Option<usize>) -> Self::SerializeSeq,
        serialize_tuple(usize) -> Self::SerializeTuple,
        serialize_tuple_struct(&'static str, usize) -> Self::SerializeTupleStruct,
        serialize_tuple_variant(&'static str, u32, &'static str, usize)
            -> Self::SerializeTupleVariant,
        serialize_map(Option<usize>) -> Self::SerializeMap,
        serialize_struct(&'static str, usize) -> Self::SerializeStruct,
        serialize_struct_variant(&'static str, u32, &'static str, usize)
            -> Self::SerializeStructVariant,
    }
}

/// Reads one value of a document.
#[derive(Clone, Copy)]
struct Reader<'de> {
    value: ValueRef<'de>,
}

impl<'de> Reader<'de> {
    /// The payload of an extended value as an integer, which the sized numbers are.
    fn integer<V: Visitor<'de>>(payload: ValueRef<'de>, visitor: V) -> Result<V::Value, Error> {
        match payload.item() {
            Item::Int(v) => visitor.visit_i64(v),
            Item::UInt(v) => visitor.visit_u64(v),
            _ => refused("an extended number without an integer in it"),
        }
    }
}

impl<'de> de::Deserializer<'de> for Reader<'de> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.value.item() {
            Item::Null | Item::Undefined | Item::Ext(ext::NONE, _) => visitor.visit_unit(),
            Item::Bool(v) => visitor.visit_bool(v),
            Item::Int(v) => visitor.visit_i64(v),
            Item::UInt(v) => visitor.visit_u64(v),
            Item::Float32(v) => visitor.visit_f32(v),
            Item::Float64(v) => visitor.visit_f64(v),
            Item::String(v) => visitor.visit_borrowed_str(v),
            Item::Data(v) => visitor.visit_borrowed_bytes(v),
            Item::Array(array) => visitor.visit_seq(Items { array, next: 0 }),
            Item::Dict(dict) => visitor.visit_map(Fields { entries: dict.iter(), value: None }),
            Item::Ext(ext::I8..=ext::U64, Some(payload)) => Self::integer(payload, visitor),
            Item::Ext(ext::F16, Some(payload)) => match payload.as_u64().map(u16::try_from) {
                Some(Ok(bits)) => visitor.visit_f64(F16(bits).to_f64()),
                _ => refused("an f16 without its bits"),
            },
            Item::Ext(ext::TABLE | ext::REGEX | ext::SET | ext::UUID, Some(payload)) => {
                Reader { value: payload }.deserialize_any(visitor)
            }
            Item::Ext(kind, _) => Err(Error::Message(format!(
                "a value of the kind {} is read as its own type or as a fleece::Value",
                ext::name(kind).unwrap_or("?")
            ))),
            Item::Invalid => Err(DecodeError::Malformed { pos: self.value.position() }.into()),
        }
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        match self.value.item() {
            Item::Null | Item::Undefined | Item::Ext(ext::NONE, _) => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        if name != TOKEN {
            return visitor.visit_newtype_struct(self);
        }
        // A value of this crate: it is handed over as a document of its own.
        let mut encoder = match self.value.doc().shared() {
            Some(shared) => Encoder::new().with_fixed_shared_keys(shared),
            None => Encoder::new(),
        };
        encoder.write_copy(self.value)?;
        visitor.visit_byte_buf(encoder.finish()?)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        match self.value.item() {
            // A variant without data is its name, one with data a dictionary of one entry.
            Item::String(variant) => visitor.visit_enum(variant.into_deserializer()),
            Item::Dict(dict) => {
                let mut entries = dict.iter();
                match (entries.next(), entries.next()) {
                    (Some((variant, value)), None) => {
                        visitor.visit_enum(Variant { variant, value })
                    }
                    _ => refused("an enum is a dictionary of one entry"),
                }
            }
            _ => refused("an enum is a string or a dictionary of one entry"),
        }
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }

    ::serde::forward_to_deserialize_any! {
        bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes byte_buf
        unit unit_struct seq tuple tuple_struct map struct identifier
    }
}

struct Items<'de> {
    array: ArrayRef<'de>,
    next: usize,
}

impl<'de> de::SeqAccess<'de> for Items<'de> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, Error> {
        let Some(value) = self.array.get(self.next) else {
            return Ok(None);
        };
        self.next += 1;
        seed.deserialize(Reader { value }).map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.array.len().saturating_sub(self.next))
    }
}

struct Fields<'de> {
    entries: DictIter<'de>,
    /// The value of the entry whose key was given last.
    value: Option<ValueRef<'de>>,
}

impl<'de> de::MapAccess<'de> for Fields<'de> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Error> {
        let Some((key, value)) = self.entries.next() else {
            return Ok(None);
        };
        self.value = Some(value);
        seed.deserialize(KeyReader { key }).map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        match self.value.take() {
            Some(value) => seed.deserialize(Reader { value }),
            None => refused("a value was asked for before its key"),
        }
    }
}

/// Reads the key of a dictionary: a string, which is also the number or the variant a map
/// was written with as its key.
struct KeyReader<'de> {
    key: &'de str,
}

/// The methods that read the key as a number of a type.
macro_rules! parsed {
    ($($method:ident => $visit:ident,)*) => {$(
        fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
            match self.key.parse() {
                Ok(number) => visitor.$visit(number),
                Err(_) => visitor.visit_borrowed_str(self.key),
            }
        }
    )*};
}

impl<'de> de::Deserializer<'de> for KeyReader<'de> {
    type Error = Error;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        visitor.visit_borrowed_str(self.key)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _: &'static str,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_enum(BorrowedStrDeserializer::new(self.key))
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_newtype_struct(self)
    }

    parsed! {
        deserialize_i8 => visit_i8,
        deserialize_i16 => visit_i16,
        deserialize_i32 => visit_i32,
        deserialize_i64 => visit_i64,
        deserialize_u8 => visit_u8,
        deserialize_u16 => visit_u16,
        deserialize_u32 => visit_u32,
        deserialize_u64 => visit_u64,
    }

    ::serde::forward_to_deserialize_any! {
        bool i128 u128 f32 f64 char str string bytes byte_buf option unit unit_struct seq
        tuple tuple_struct map struct identifier ignored_any
    }
}

/// A variant with data: its name and what it holds.
struct Variant<'de> {
    variant: &'de str,
    value: ValueRef<'de>,
}

impl<'de> de::EnumAccess<'de> for Variant<'de> {
    type Error = Error;
    type Variant = Reader<'de>;

    fn variant_seed<V: DeserializeSeed<'de>>(
        self,
        seed: V,
    ) -> Result<(V::Value, Reader<'de>), Error> {
        let name: StrDeserializer<'de, Error> = self.variant.into_deserializer();
        Ok((seed.deserialize(name)?, Reader { value: self.value }))
    }
}

impl<'de> de::VariantAccess<'de> for Reader<'de> {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        seed.deserialize(self)
    }

    fn tuple_variant<V: Visitor<'de>>(self, _: usize, visitor: V) -> Result<V::Value, Error> {
        self.deserialize_any(visitor)
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        _: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.deserialize_any(visitor)
    }
}

/// The bytes of a document, written as bytes.
struct Document<'a>(&'a [u8]);

impl Serialize for Document<'_> {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(self.0)
    }
}

impl Serialize for Value {
    fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let document = encode(self).map_err(ser::Error::custom)?;
        serializer.serialize_newtype_struct(TOKEN, &Document(&document))
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = Value;

    fn expecting(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str("the bytes of a Fleece document")
    }

    fn visit_bytes<E: de::Error>(self, document: &[u8]) -> Result<Value, E> {
        decode(document).map_err(E::custom)
    }

    fn visit_newtype_struct<D: de::Deserializer<'de>>(self, inner: D) -> Result<Value, D::Error> {
        inner.deserialize_bytes(self)
    }

    fn visit_seq<A: de::SeqAccess<'de>>(self, mut bytes: A) -> Result<Value, A::Error> {
        // A format without bytes of its own gives them one by one.
        let mut document = Vec::new();
        while let Some(byte) = bytes.next_element()? {
            document.push(byte);
        }
        decode(&document).map_err(de::Error::custom)
    }
}

impl<'de> Deserialize<'de> for Value {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_newtype_struct(TOKEN, ValueVisitor)
    }
}

/// A type that is one variant of [`Value`] is written and read as that value.
macro_rules! as_value {
    ($($type:ty: $name:literal, $wrap:expr, $unwrap:pat => $inner:expr;)*) => {$(
        impl Serialize for $type {
            fn serialize<S: ser::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let wrap: fn($type) -> Value = $wrap;
                wrap(self.clone()).serialize(serializer)
            }
        }

        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                match Value::deserialize(deserializer)? {
                    $unwrap => Ok($inner),
                    _ => Err(de::Error::custom(concat!("not a value of the type ", $name))),
                }
            }
        }
    )*};
}

as_value! {
    Number: "number", Value::Number, Value::Number(v) => v;
    Decimal: "decimal", |v| Value::Number(Number::Decimal(v)),
        Value::Number(Number::Decimal(v)) => v;
    BigDecimal: "bigdecimal", |v| Value::Number(Number::BigDecimal(v)),
        Value::Number(Number::BigDecimal(v)) => v;
    F16: "f16", |v| Value::Number(Number::F16(v)), Value::Number(Number::F16(v)) => v;
    Uuid: "uuid", Value::Uuid, Value::Uuid(v) => v;
    Duration: "duration", Value::Duration, Value::Duration(v) => v;
    Datetime: "datetime", Value::Datetime, Value::Datetime(v) => v;
    Timestamp: "timestamp", Value::Timestamp, Value::Timestamp(v) => v;
    Date: "date", Value::Date, Value::Date(v) => v;
    Time: "time", Value::Time, Value::Time(v) => v;
    Interval: "interval", Value::Interval, Value::Interval(v) => v;
    RecordId: "record", Value::RecordId, Value::RecordId(v) => v;
    File: "file", Value::File, Value::File(v) => v;
    Geometry: "geometry", Value::Geometry, Value::Geometry(v) => v;
    Range: "range", |v| Value::Range(Box::new(v)), Value::Range(v) => *v;
}
