//! The Fleece binary format, extended with the value types of SurrealDB.
//!
//! [Fleece](https://github.com/couchbase/fleece) is Couchbase's encoding of JSON-like data
//! that is read in place: a value is a position in the bytes, an array item is found in
//! constant time and a dictionary key by binary search, and nothing is parsed or copied.
//!
//! The crate has these parts, each usable on its own:
//!
//! - [`Doc`], [`ValueRef`], [`ArrayRef`], [`DictRef`]: reading bytes in place, after
//!   [`Doc::validate`] when they are not trusted;
//! - [`Encoder`]: writing a document value by value;
//! - [`SharedKeys`]: dictionary keys stored as small numbers across many documents;
//! - [`Value`] with [`encode`] and [`decode`]: an owned value of any type SurrealDB has;
//! - [`delta`]: the difference between two values, as bytes to append or as a value;
//! - [`compact`]: a document that grew by appended deltas written anew without what they
//!   left behind;
//! - [`update`] with [`Patch`], and [`extract`]: a document changed by paths, and a part of
//!   one, without decoding it;
//! - [`Kind`]: the kinds of SurrealDB, and whether a value in a document is of one;
//! - [`key`]: values as keys of an ordered store, for indexes;
//! - [`serde`](mod@crate::serde) (feature `serde`): the types of an application to documents and back;
//! - [`KeyPath`]: `foo.bar[2].baz` into a document;
//! - [`json`] (feature `json`): JSON to values and back;
//! - [`dump`]: a listing of a document by address.
//!
//! # The format
//!
//! Every value of standard Fleece (null, booleans, integers, floats, strings, binary data,
//! arrays, dictionaries) is written byte for byte as Couchbase's implementation writes it,
//! and its documents are read. The types Fleece lacks are an extension: the byte `0x31`, a
//! code Fleece leaves unused, then the kind (see [`ext`]), then one standard value that is
//! the payload. A document that holds such a value is not readable by Couchbase's Fleece.
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use fleece::{Number, Object, Uuid, Value};
//!
//! let user = Value::Object(Object::from([
//!     ("name".to_owned(), Value::from("Milou")),
//!     ("id".to_owned(), Value::Uuid(Uuid([7; 16]))),
//!     ("age".to_owned(), Value::Number(Number::U8(9))),
//! ]));
//! let bytes = fleece::encode(&user)?;
//!
//! // Read one field in place.
//! let root = fleece::Doc::new(&bytes)?.root();
//! assert_eq!(root.as_dict().and_then(|dict| dict.get("name")?.as_str()), Some("Milou"));
//! // Or take the whole value back.
//! assert_eq!(fleece::decode(&bytes)?, user);
//! # Ok(())
//! # }
//! ```

//!
//! # Documents in a store
//!
//! A key-value store keeps a document as one value. What follows from that:
//!
//! - **Read in place.** A store lends the bytes of a value (a closure over `&[u8]`, a
//!   cursor). [`Doc::trusted`] over them reads a field without a copy and without a pass
//!   over the document; nothing is allocated. Reading never panics and never leaves the
//!   bytes, whatever they are.
//! - **Check at the door.** [`Doc::new`] passes over the whole document once. Do it where
//!   bytes come in from outside; what the application wrote itself needs no check on every
//!   read.
//! - **Change by [`update`].** The store writes a value whole on every change, so an
//!   appended delta saves nothing there: what a change costs is making the new bytes, which
//!   [`update`] does from the old ones without a [`Value`] in between. Appended deltas are
//!   for sending a change and for logs.
//! - **Index by [`key`].** The bytes of documents do not sort; the key of an index entry is
//!   [`key::encode_key`] of the indexed field, followed by the key of the document.
//! - **Keep the version.** Documents carry no version. Store [`FORMAT_VERSION`] once beside
//!   them.
//! - **Shared keys travel with the documents.** A table of [`SharedKeys`] is stored in the
//!   same transaction as the document that grew it, and a write that failed takes its keys
//!   back with [`SharedKeys::truncate`].
//! - **Large binary data stays outside.** Every change of a document writes all of it.
//!
//! ```
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! use fleece::{Doc, Object, Patch, Value};
//!
//! let dog = |visits: i64| {
//!     let fields = [("name".to_owned(), Value::from("Milou")), ("visits".to_owned(), visits.into())];
//!     Value::Object(Object::from(fields))
//! };
//! // What a store lends for a key, and what the application does with it.
//! let stored = fleece::encode(&dog(41))?;
//! let doc = Doc::trusted(&stored);
//! let visits = doc.root().as_dict().and_then(|dict| dict.get("visits")?.as_i64());
//!
//! let (path, more) = ("visits".parse()?, Value::from(visits.unwrap_or(0) + 1));
//! let mut patch = Patch::new();
//! patch.set(&path, &more);
//! let to_store = fleece::update(&doc, &patch)?;
//!
//! // The entry of an index over `name`: bytes that sort as the names do.
//! let index_key = fleece::key::encode_key(&Value::from("Milou"));
//! # assert_eq!(fleece::decode(&to_store)?, dog(42));
//! # assert!(index_key < fleece::key::encode_key(&Value::from("Tintin")));
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]

mod codec;
mod compact;
pub mod delta;
mod dump;
mod encoder;
mod error;
mod format;
mod hash;
#[cfg(feature = "json")]
pub mod json;
pub mod key;
mod keypath;
mod kind;
mod reader;
#[cfg(feature = "serde")]
pub mod serde;
mod shared_keys;
mod update;
mod value;

pub use codec::{decode, encode};
pub use compact::compact;
pub use dump::dump;
pub use encoder::Encoder;
pub use error::{DecodeError, DeltaError, EncodeError};
pub use format::{FORMAT_VERSION, MAX_DEPTH, ext};
pub use keypath::{KeyPath, KeyPathError};
pub use kind::{GeometryKind, Kind, KindLiteral};
pub use reader::{ArrayRef, DictIter, DictRef, Doc, Item, ValueRef};
pub use shared_keys::SharedKeys;
pub use update::{Patch, extract, update};
pub use value::{
    BigDecimal, Date, Datetime, Decimal, Duration, F16, File, Geometry, Interval, Number, Object,
    ParseError, Point, Polygon, Range, RecordId, RecordIdKey, RecordIdKeyRange, Time, Timestamp,
    Uuid, Value,
};
