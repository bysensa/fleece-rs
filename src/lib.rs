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
mod keypath;
mod reader;
mod shared_keys;
mod value;

pub use codec::{decode, encode};
pub use compact::compact;
pub use dump::dump;
pub use encoder::Encoder;
pub use error::{DecodeError, DeltaError, EncodeError};
pub use format::{MAX_DEPTH, ext};
pub use keypath::{KeyPath, KeyPathError};
pub use reader::{ArrayRef, DictIter, DictRef, Doc, Item, ValueRef};
pub use shared_keys::SharedKeys;
pub use value::{
    BigDecimal, Date, Datetime, Decimal, Duration, F16, File, Geometry, Interval, Number, Object,
    ParseError, Point, Polygon, Range, RecordId, RecordIdKey, RecordIdKeyRange, Time, Timestamp,
    Uuid, Value,
};
