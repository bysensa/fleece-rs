//! The errors of the crate.

use std::fmt::{self, Display, Formatter};

/// Why bytes were refused as a Fleece document, or a value in them as a typed value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// The data is shorter than one value or its length is odd.
    Truncated,
    /// The value at `pos` is cut short, misaligned or uses a reserved encoding.
    Malformed { pos: usize },
    /// The pointer at `pos` leaves the data, points forward or at another pointer.
    BadPointer { pos: usize },
    /// The string at `pos` is not UTF-8.
    InvalidUtf8 { pos: usize },
    /// The extended value at `pos` has a kind this version does not know.
    UnknownExt { pos: usize, kind: u8 },
    /// The payload of the extended value at `pos` is not what its kind takes.
    BadPayload { pos: usize, kind: u8 },
    /// A key of the dictionary at `pos` is neither a string nor a shared key.
    BadKey { pos: usize },
    /// The keys of the dictionary at `pos` are not in ascending order, or one repeats.
    UnsortedKeys { pos: usize },
    /// The dictionary at `pos` has a shared key the document's shared keys do not hold.
    MissingSharedKey { pos: usize, key: i64 },
    /// `undefined` at `pos`, outside a dictionary that inherits.
    Undefined { pos: usize },
    /// Collections nest deeper than [`crate::MAX_DEPTH`].
    TooDeep,
    /// The document expands, through values shared by pointers, to more values than allowed.
    TooManyValues,
}

impl Display for DecodeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("the data is too short or of odd length"),
            Self::Malformed { pos } => write!(f, "malformed value at {pos:#x}"),
            Self::BadPointer { pos } => write!(f, "bad pointer at {pos:#x}"),
            Self::InvalidUtf8 { pos } => write!(f, "the string at {pos:#x} is not UTF-8"),
            Self::UnknownExt { pos, kind } => {
                write!(f, "unknown kind {kind} of the extended value at {pos:#x}")
            }
            Self::BadPayload { pos, kind } => {
                write!(f, "wrong payload for kind {kind} of the extended value at {pos:#x}")
            }
            Self::BadKey { pos } => write!(f, "bad key in the dictionary at {pos:#x}"),
            Self::UnsortedKeys { pos } => {
                write!(f, "the keys of the dictionary at {pos:#x} are not sorted and unique")
            }
            Self::MissingSharedKey { pos, key } => {
                write!(f, "shared key {key} of the dictionary at {pos:#x} is not known")
            }
            Self::Undefined { pos } => write!(f, "undefined value at {pos:#x}"),
            Self::TooDeep => f.write_str("collections nest too deep"),
            Self::TooManyValues => f.write_str("the document expands to too many values"),
        }
    }
}

impl std::error::Error for DecodeError {}

/// Why an [`crate::Encoder`] refused a call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncodeError {
    /// A value was written where the dictionary takes a key.
    ExpectedKey,
    /// A key was written outside a dictionary or where it takes a value.
    UnexpectedKey,
    /// `end_array` or `end_dict` does not match the collection that is open.
    Unbalanced,
    /// A second value was written at the top level.
    MultipleRoots,
    /// `finish` was called with a collection open or nothing written.
    Incomplete,
    /// The call is not allowed right after `begin_ext`.
    ExtPayload,
    /// The kind given to `begin_ext` takes no payload or is not known.
    BadExtKind(u8),
    /// The dictionary has this key twice.
    DuplicateKey(String),
    /// Collections nest deeper than [`crate::MAX_DEPTH`].
    TooDeep,
    /// A value lies farther back than a pointer reaches.
    TooLarge,
    /// The value given to `write_ref` is not in the base document.
    NotInBase,
    /// The base document has an odd length.
    BadBase,
    /// The value given to `write_copy` is not a well formed value.
    InvalidValue,
}

impl Display for EncodeError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExpectedKey => f.write_str("the dictionary takes a key here"),
            Self::UnexpectedKey => f.write_str("a key is not taken here"),
            Self::Unbalanced => f.write_str("the end does not match the open collection"),
            Self::MultipleRoots => f.write_str("a document holds one top-level value"),
            Self::Incomplete => f.write_str("the document is not complete"),
            Self::ExtPayload => f.write_str("an extended value takes one payload value here"),
            Self::BadExtKind(kind) => write!(f, "kind {kind} takes no payload or is unknown"),
            Self::DuplicateKey(key) => write!(f, "the dictionary has the key {key:?} twice"),
            Self::TooDeep => f.write_str("collections nest too deep"),
            Self::TooLarge => f.write_str("a value lies farther back than a pointer reaches"),
            Self::NotInBase => f.write_str("the value is not in the base document"),
            Self::BadBase => f.write_str("the base document has an odd length"),
            Self::InvalidValue => f.write_str("the value to copy is not well formed"),
        }
    }
}

impl std::error::Error for EncodeError {}

/// Why a delta could not be made or applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeltaError {
    /// The base document was refused.
    Decode(DecodeError),
    /// The delta could not be written.
    Encode(EncodeError),
    /// The delta is not of the form [`crate::delta::diff`] gives.
    Malformed,
    /// The delta does not fit the value it is applied to.
    Mismatch,
}

impl Display for DeltaError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(e) => write!(f, "the base document: {e}"),
            Self::Encode(e) => write!(f, "writing the delta: {e}"),
            Self::Malformed => f.write_str("the delta is malformed"),
            Self::Mismatch => f.write_str("the delta does not fit the value"),
        }
    }
}

impl std::error::Error for DeltaError {}

impl From<DecodeError> for DeltaError {
    fn from(e: DecodeError) -> Self {
        Self::Decode(e)
    }
}

impl From<EncodeError> for DeltaError {
    fn from(e: EncodeError) -> Self {
        Self::Encode(e)
    }
}
