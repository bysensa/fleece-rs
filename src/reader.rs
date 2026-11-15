//! Reading a document in place: values are positions in the bytes, nothing is copied.

use std::collections::{BTreeMap, HashMap};
use std::fmt::{self, Debug, Formatter};

use crate::error::DecodeError;
use crate::format::*;
use crate::hash::Seeds;
use crate::shared_keys::SharedKeys;

/// The most values a document of any size may expand to once every pointer is followed.
/// Values shared by pointers count once per use, so that a small document cannot stand for
/// an endless one. A document is held to less than this: to one value for every two of its
/// bytes, see [`Doc::validate`].
const MAX_VALUES: u64 = 1 << 26;

/// An encoded document: its bytes, the base it was appended to, and its shared keys.
///
/// The bytes of the base and of the document form one address space, the base first, as if
/// the document had been appended to it.
#[derive(Clone, Copy)]
pub struct Doc<'a> {
    base: &'a [u8],
    data: &'a [u8],
    shared: Option<&'a SharedKeys>,
}

impl Debug for Doc<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Doc({} bytes over a base of {})", self.data.len(), self.base.len())
    }
}

/// A value as its header says it, before any pointer in it is followed.
#[derive(Clone, Copy)]
pub(crate) enum Raw<'a> {
    Int(i64),
    UInt(u64),
    F32(f32),
    F64(f64),
    Null,
    Undefined,
    Bool(bool),
    Str(&'a [u8]),
    Data(&'a [u8]),
    Coll { dict: bool, wide: bool, count: usize, first: usize },
    Ext(u8),
    Pointer,
}

impl<'a> Doc<'a> {
    /// A document over bytes that are checked first; see [`Doc::validate`].
    pub fn new(data: &'a [u8]) -> Result<Self, DecodeError> {
        let doc = Self::trusted(data);
        doc.validate()?;
        Ok(doc)
    }

    /// A document over bytes that are not checked. Reading never panics and never leaves the
    /// bytes, whatever they are, but a value that is not well formed reads as
    /// [`Item::Invalid`].
    pub fn trusted(data: &'a [u8]) -> Self {
        Self { base: &[], data, shared: None }
    }

    /// The document as a delta over `base`: its pointers may reach back into the base.
    pub fn with_base(mut self, base: &'a [u8]) -> Self {
        self.base = base;
        self
    }

    /// The document with the shared keys its dictionaries were written with.
    pub fn with_shared_keys(mut self, shared: &'a SharedKeys) -> Self {
        self.shared = Some(shared);
        self
    }

    /// The root value.
    pub fn root(&self) -> ValueRef<'a> {
        ValueRef { doc: *self, pos: self.root_pos().unwrap_or(usize::MAX) }
    }

    /// The value at a position [`ValueRef::position`] gave for this document.
    pub fn value_at(&self, pos: usize) -> ValueRef<'a> {
        ValueRef { doc: *self, pos }
    }

    /// The shared keys the document was given.
    pub(crate) fn shared(&self) -> Option<&'a SharedKeys> {
        self.shared
    }

    /// The length of base and document together.
    pub(crate) fn end(&self) -> usize {
        self.base.len() + self.data.len()
    }

    /// The bytes from `pos` to the end of the base or of the data, whichever holds `pos`.
    pub(crate) fn tail(&self, pos: usize) -> &'a [u8] {
        match pos.checked_sub(self.base.len()) {
            None => self.base.get(pos..).unwrap_or(&[]),
            Some(p) => self.data.get(p..).unwrap_or(&[]),
        }
    }

    pub(crate) fn is_pointer(&self, pos: usize) -> bool {
        self.tail(pos).first().is_some_and(|b| b & 0x80 != 0)
    }

    /// Where the pointer at `addr` leads, and whether it is marked as leaving the document.
    pub(crate) fn pointer(&self, addr: usize, wide: bool) -> Option<(usize, bool)> {
        let b = self.tail(addr);
        let (offset, external) = if wide {
            let v = u32::from_be_bytes(b.get(..4)?.try_into().ok()?);
            ((v & 0x3FFF_FFFF) as usize * 2, v & 0x4000_0000 != 0)
        } else {
            let v = u16::from_be_bytes(b.get(..2)?.try_into().ok()?);
            ((v & 0x3FFF) as usize * 2, v & 0x4000 != 0)
        };
        if offset == 0 {
            return None;
        }
        Some((addr.checked_sub(offset)?, external))
    }

    /// The position of the value the slot at `addr` of a collection holds.
    fn resolve(&self, addr: usize, wide: bool) -> Option<usize> {
        if self.is_pointer(addr) { Some(self.pointer(addr, wide)?.0) } else { Some(addr) }
    }

    fn root_pos(&self) -> Option<usize> {
        let end = self.end().checked_sub(2).filter(|_| self.data.len() >= 2)?;
        if !self.is_pointer(end) {
            return Some(end);
        }
        // A root farther back than a narrow pointer reaches is found through a wide one.
        let target = self.pointer(end, false)?.0;
        if self.is_pointer(target) { Some(self.pointer(target, true)?.0) } else { Some(target) }
    }

    /// The value at `pos` and the bytes it takes, an extended value's payload not counted.
    pub(crate) fn raw(&self, pos: usize) -> Option<(Raw<'a>, usize)> {
        let b = self.tail(pos);
        let (b0, b1) = (*b.first()?, *b.get(1)?);
        if b0 & 0x80 != 0 {
            return Some((Raw::Pointer, 2));
        }
        let (raw, size) = match b0 >> 4 {
            TAG_SHORT_INT => {
                let v = (u16::from(b0 & 0x0F) << 8 | u16::from(b1)) << 4;
                (Raw::Int(i64::from(v as i16 >> 4)), 2)
            }
            TAG_INT => {
                let n = usize::from(b0 & 7) + 1;
                let bytes = b.get(1..1 + n)?;
                let unsigned = b0 & 8 != 0;
                let fill = if !unsigned && bytes[n - 1] & 0x80 != 0 { 0xFF } else { 0 };
                let mut buf = [fill; 8];
                buf[..n].copy_from_slice(bytes);
                let v = u64::from_le_bytes(buf);
                let raw = match i64::try_from(v) {
                    Err(_) if unsigned => Raw::UInt(v),
                    _ => Raw::Int(v as i64),
                };
                (raw, even(1 + n))
            }
            TAG_FLOAT => match b0 {
                FLOAT_32 | FLOAT_32_DOUBLE => {
                    let v = f32::from_le_bytes(b.get(2..6)?.try_into().ok()?);
                    (if b0 == FLOAT_32 { Raw::F32(v) } else { Raw::F64(f64::from(v)) }, 6)
                }
                FLOAT_64 => (Raw::F64(f64::from_le_bytes(b.get(2..10)?.try_into().ok()?)), 10),
                _ => return None,
            },
            TAG_SPECIAL => match b0 {
                SPECIAL_NULL => (Raw::Null, 2),
                SPECIAL_FALSE => (Raw::Bool(false), 2),
                SPECIAL_TRUE => (Raw::Bool(true), 2),
                SPECIAL_UNDEFINED => (Raw::Undefined, 2),
                SPECIAL_EXT => (Raw::Ext(b1), 2),
                _ => return None,
            },
            tag @ (TAG_STRING | TAG_DATA) => {
                let mut len = usize::from(b0 & 0x0F);
                let mut start = 1;
                if len == 15 {
                    let (n, used) = get_varint(&b[1..])?;
                    len = usize::try_from(n).ok()?;
                    start += used;
                }
                let bytes = b.get(start..start.checked_add(len)?)?;
                let raw = if tag == TAG_STRING { Raw::Str(bytes) } else { Raw::Data(bytes) };
                (raw, even(start + len))
            }
            tag => {
                let dict = tag == TAG_DICT;
                let wide = b0 & 8 != 0;
                let mut count = usize::from(b0 & 7) << 8 | usize::from(b1);
                let mut header = 2;
                if count == LONG_COUNT {
                    let (extra, used) = get_varint(b.get(2..)?)?;
                    count = count.checked_add(usize::try_from(extra).ok()?)?;
                    header += even(used);
                }
                let slots = if dict { count.checked_mul(2)? } else { count };
                let size = header.checked_add(slots.checked_mul(if wide { 4 } else { 2 })?)?;
                (Raw::Coll { dict, wide, count, first: pos.checked_add(header)? }, size)
            }
        };
        (size <= even(b.len())).then_some((raw, size))
    }

    /// The bytes the value at `pos` takes, the payload of an extended value included.
    pub(crate) fn size_of(&self, pos: usize) -> Option<usize> {
        match self.raw(pos)? {
            (Raw::Ext(kind), _) if kind != ext::NONE => match self.raw(pos + 2)? {
                (Raw::Ext(_) | Raw::Pointer, _) => None,
                (_, size) => Some(2 + size),
            },
            (_, size) => Some(size),
        }
    }

    /// Checks the whole document: every value reachable from the root lies inside the bytes,
    /// every pointer leads back to a value, strings are UTF-8, dictionary keys are sorted and
    /// unique, extended values have a known kind and a payload of its form, nothing is
    /// undefined but an entry a dictionary deletes from the one it inherits, and nesting and
    /// size stay within limits. A document that passes is one [`crate::decode`] takes.
    ///
    /// The document may expand to one value for every two of its bytes, the base included.
    /// A value takes two bytes at least where it is used, so a document without a
    /// collection that many pointers lead to stays under that; pointers that would make a
    /// small document stand for a large one go over it. What a dictionary hides in the one
    /// it inherits from is not counted: no reader comes to it.
    pub fn validate(&self) -> Result<(), DecodeError> {
        if self.data.len() < 2 || self.data.len() & 1 == 1 || self.base.len() & 1 == 1 {
            return Err(DecodeError::Truncated);
        }
        let end = self.end() - 2;
        let mut validator = Validator {
            doc: self,
            // Room for a collection in every 64 bytes, which few documents exceed.
            seen: HashMap::with_capacity_and_hasher(self.end() / 64, Seeds::random()),
            strings: vec![0; self.end() / 128 + 1],
            most: MAX_VALUES.min(self.end() as u64 / 2),
        };
        let (root, limit) = if self.is_pointer(end) {
            let target = validator.pointer(end, false, true)?;
            let root = match self.is_pointer(target) {
                true => validator.pointer(target, true, false)?,
                false => target,
            };
            (root, usize::MAX)
        } else {
            (end, 2)
        };
        validator.value(root, 0, true, limit).map(|_| ())
    }
}

struct Validator<'d, 'a> {
    doc: &'d Doc<'a>,
    /// The collections reached by a pointer and already checked: how many values each
    /// expands to and how deep the collections in it nest.
    seen: HashMap<usize, (u64, usize), Seeds>,
    /// A bit for each even position: the string a pointer leads to there is UTF-8.
    strings: Vec<u64>,
    /// The most values the document may expand to.
    most: u64,
}

impl<'a> Validator<'_, 'a> {
    /// Checks that a string is UTF-8, once for a string that many pointers lead to.
    fn utf8(&mut self, pos: usize, bytes: &[u8], shared: bool) -> Result<(), DecodeError> {
        let (word, bit) = (pos / 128, 1 << (pos / 2 % 64));
        if shared && self.strings.get(word).is_some_and(|bits| bits & bit != 0) {
            return Ok(());
        }
        std::str::from_utf8(bytes).map_err(|_| DecodeError::InvalidUtf8 { pos })?;
        if shared && let Some(bits) = self.strings.get_mut(word) {
            *bits |= bit;
        }
        Ok(())
    }

    fn pointer(&self, addr: usize, wide: bool, to_pointer: bool) -> Result<usize, DecodeError> {
        let bad = DecodeError::BadPointer { pos: addr };
        let (target, external) = self.doc.pointer(addr, wide).ok_or(bad.clone())?;
        // A pointer marked as leaving the document has to land in the base.
        if (external && target >= self.doc.base.len())
            || self.doc.tail(target).is_empty()
            || (!to_pointer && self.doc.is_pointer(target))
        {
            return Err(bad);
        }
        Ok(target)
    }

    /// Checks the value at `pos`, `depth` collections down, and gives how many values it
    /// expands to and how deep the collections in it nest. `shared` says a pointer led
    /// here, so the value may be met again; `limit` is the size of the slot it lies in.
    fn value(
        &mut self,
        pos: usize,
        depth: usize,
        shared: bool,
        limit: usize,
    ) -> Result<(u64, usize), DecodeError> {
        let malformed = DecodeError::Malformed { pos };
        if pos & 1 == 1 {
            return Err(malformed);
        }
        let (raw, size) = self.doc.raw(pos).ok_or(malformed.clone())?;
        // An extended value is its payload, which follows it, and one value more.
        let (at, raw, size, kind) = match raw {
            Raw::Ext(kind) if kind > ext::MAX => {
                return Err(DecodeError::UnknownExt { pos, kind });
            }
            Raw::Ext(kind) if kind != ext::NONE => match self.doc.raw(pos + 2) {
                None | Some((Raw::Ext(_) | Raw::Pointer, _)) => {
                    return Err(DecodeError::BadPayload { pos, kind });
                }
                Some((payload, payload_size)) => (pos + 2, payload, payload_size + 2, Some(kind)),
            },
            _ => (pos, raw, size, None),
        };
        if size > limit {
            return Err(malformed);
        }
        let extended = u64::from(kind.is_some());
        let (values, height) = match raw {
            Raw::Pointer => return Err(malformed),
            // Undefined is how a dictionary deletes an entry of the one it inherits from,
            // which `collection` lets through; anywhere else no value can be made of it.
            Raw::Undefined => return Err(DecodeError::Undefined { pos }),
            Raw::Str(bytes) => {
                self.utf8(at, bytes, shared)?;
                (1 + extended, 0)
            }
            Raw::Coll { dict, wide, count, first } => {
                // A collection many pointers lead to is checked once: what it expands to
                // is remembered, which also keeps the work in step with the size of the
                // document whatever its pointers do.
                if shared && let Some(&(values, height)) = self.seen.get(&pos) {
                    if depth + height > MAX_DEPTH {
                        return Err(DecodeError::TooDeep);
                    }
                    return Ok((values, height));
                }
                let (values, height) = self.collection(at, depth, dict, wide, count, first)?;
                if shared {
                    self.seen.insert(pos, (values + extended, height));
                }
                (values + extended, height)
            }
            _ => (1 + extended, 0),
        };
        // The values inside the payload are checked by now, so what is left to ask is
        // whether the payload has the form of its kind.
        if let Some(kind) = kind {
            crate::codec::check_payload(kind, self.doc.value_at(at), pos)?;
        }
        Ok((values, height))
    }

    fn collection(
        &mut self,
        pos: usize,
        depth: usize,
        dict: bool,
        wide: bool,
        count: usize,
        first: usize,
    ) -> Result<(u64, usize), DecodeError> {
        if depth >= MAX_DEPTH {
            return Err(DecodeError::TooDeep);
        }
        let width = if wide { 4 } else { 2 };
        let (mut values, mut height) = (1u64, 1);
        let mut previous = None;
        let mut inherits = false;
        for i in 0..if dict { 2 * count } else { count } {
            let addr = first + i * width;
            let by_pointer = self.doc.is_pointer(addr);
            // A value that is not pointed at has to fit the slot it lies in.
            let (target, limit) = match by_pointer {
                true => (self.pointer(addr, wide, false)?, usize::MAX),
                false => (addr, width),
            };
            if dict && i % 2 == 0 {
                let Some((raw, _)) = self.doc.raw(target).filter(|(_, size)| *size <= limit) else {
                    return Err(match by_pointer {
                        true => DecodeError::BadKey { pos },
                        false => DecodeError::Malformed { pos: addr },
                    });
                };
                let key = match raw {
                    Raw::Int(PARENT_KEY) if i == 0 => {
                        inherits = true;
                        Key::Int(PARENT_KEY)
                    }
                    Raw::Int(key) => {
                        let known = u16::try_from(key)
                            .ok()
                            .and_then(|key| self.doc.shared?.decode(key))
                            .is_some();
                        if !known {
                            return Err(DecodeError::MissingSharedKey { pos, key });
                        }
                        Key::Int(key)
                    }
                    Raw::Str(bytes) => {
                        self.utf8(target, bytes, by_pointer)?;
                        Key::Str(bytes)
                    }
                    _ => return Err(DecodeError::BadKey { pos }),
                };
                if previous.is_some_and(|previous| previous >= key) {
                    return Err(DecodeError::UnsortedKeys { pos });
                }
                previous = Some(key);
                values += 1;
                continue;
            }
            // An entry deleted from the dictionary inherited from.
            if inherits && i > 1 && matches!(self.doc.raw(target), Some((Raw::Undefined, _))) {
                values += 1;
                continue;
            }
            let (inner, inner_height) = self.value(target, depth + 1, by_pointer, limit)?;
            // The entry a dictionary inherits through has to lead to a dictionary.
            if inherits
                && i == 1
                && !matches!(self.doc.raw(target), Some((Raw::Coll { dict: true, .. }, _)))
            {
                return Err(DecodeError::BadKey { pos });
            }
            values = values.saturating_add(inner);
            height = height.max(inner_height + 1);
        }
        if inherits {
            // What the dictionary inherited from holds under a key of this one is hidden:
            // it was counted with that dictionary, and no reader comes to it.
            let own = DictRef { doc: *self.doc, first, count, wide };
            for entry in 1..count {
                if let Some(hidden) = own.hidden(entry) {
                    let (inner, _) = self.value(hidden.pos, depth + 1, true, usize::MAX)?;
                    values = values.saturating_sub(1 + inner);
                }
            }
        }
        if values > self.most {
            return Err(DecodeError::TooManyValues);
        }
        Ok((values, height))
    }
}

/// A value inside a document.
#[derive(Clone, Copy)]
pub struct ValueRef<'a> {
    doc: Doc<'a>,
    pos: usize,
}

impl Debug for ValueRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "ValueRef({:#x}: {:?})", self.pos, self.item())
    }
}

/// What a [`ValueRef`] holds.
#[derive(Clone, Copy, Debug)]
pub enum Item<'a> {
    Null,
    /// Fleece's `undefined`: a key deleted from a dictionary that inherits.
    Undefined,
    Bool(bool),
    Int(i64),
    /// An unsigned integer above `i64::MAX`.
    UInt(u64),
    Float32(f32),
    Float64(f64),
    String(&'a str),
    Data(&'a [u8]),
    Array(ArrayRef<'a>),
    Dict(DictRef<'a>),
    /// An extended value: its kind (see [`crate::ext`]) and its payload, which only the kind
    /// [`crate::ext::NONE`] lacks.
    Ext(u8, Option<ValueRef<'a>>),
    /// Bytes that are not a value. A validated document has none.
    Invalid,
}

impl<'a> ValueRef<'a> {
    /// The document the value is in.
    pub fn doc(&self) -> Doc<'a> {
        self.doc
    }

    /// The position of the value in the address space of base and document.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// What the value holds.
    pub fn item(&self) -> Item<'a> {
        let Some((raw, _)) = self.doc.raw(self.pos) else {
            return Item::Invalid;
        };
        match raw {
            Raw::Int(v) => Item::Int(v),
            Raw::UInt(v) => Item::UInt(v),
            Raw::F32(v) => Item::Float32(v),
            Raw::F64(v) => Item::Float64(v),
            Raw::Null => Item::Null,
            Raw::Undefined => Item::Undefined,
            Raw::Bool(v) => Item::Bool(v),
            Raw::Str(bytes) => std::str::from_utf8(bytes).map_or(Item::Invalid, Item::String),
            Raw::Data(bytes) => Item::Data(bytes),
            Raw::Coll { dict, wide, count, first } => {
                let doc = self.doc;
                if dict {
                    Item::Dict(DictRef { doc, first, count, wide })
                } else {
                    Item::Array(ArrayRef { doc, first, count, wide })
                }
            }
            Raw::Ext(ext::NONE) => Item::Ext(ext::NONE, None),
            Raw::Ext(kind) => Item::Ext(kind, Some(ValueRef { doc: self.doc, pos: self.pos + 2 })),
            Raw::Pointer => Item::Invalid,
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self.item(), Item::Null)
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Item::Bool(v) = self.item() { Some(v) } else { None }
    }

    /// The value as an integer, when it is one that fits.
    pub fn as_i64(&self) -> Option<i64> {
        if let Item::Int(v) = self.item() { Some(v) } else { None }
    }

    /// The value as an unsigned integer, when it is one that is not negative.
    pub fn as_u64(&self) -> Option<u64> {
        match self.item() {
            Item::Int(v) => u64::try_from(v).ok(),
            Item::UInt(v) => Some(v),
            _ => None,
        }
    }

    /// The value as a float, when it is a float of either width.
    pub fn as_f64(&self) -> Option<f64> {
        match self.item() {
            Item::Float32(v) => Some(f64::from(v)),
            Item::Float64(v) => Some(v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&'a str> {
        if let Item::String(v) = self.item() { Some(v) } else { None }
    }

    pub fn as_data(&self) -> Option<&'a [u8]> {
        if let Item::Data(v) = self.item() { Some(v) } else { None }
    }

    pub fn as_array(&self) -> Option<ArrayRef<'a>> {
        if let Item::Array(v) = self.item() { Some(v) } else { None }
    }

    pub fn as_dict(&self) -> Option<DictRef<'a>> {
        if let Item::Dict(v) = self.item() { Some(v) } else { None }
    }
}

/// An array inside a document.
#[derive(Clone, Copy)]
pub struct ArrayRef<'a> {
    doc: Doc<'a>,
    first: usize,
    count: usize,
    wide: bool,
}

impl Debug for ArrayRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Array[{}]", self.count)
    }
}

impl<'a> ArrayRef<'a> {
    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// The item at `index`, in constant time.
    pub fn get(&self, index: usize) -> Option<ValueRef<'a>> {
        if index >= self.count {
            return None;
        }
        let addr = self.first + index * if self.wide { 4 } else { 2 };
        Some(ValueRef { doc: self.doc, pos: self.doc.resolve(addr, self.wide)? })
    }

    /// The items in order. An item that is not well formed reads as [`Item::Invalid`].
    pub fn iter(&self) -> impl ExactSizeIterator<Item = ValueRef<'a>> + use<'a> {
        let array = *self;
        let invalid = ValueRef { doc: self.doc, pos: usize::MAX };
        (0..self.count).map(move |index| array.get(index).unwrap_or(invalid))
    }
}

/// A key as a dictionary stores it. The order of the variants and of their contents is the
/// order of keys in a dictionary: shared keys first, then strings by their bytes.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Key<'a> {
    Int(i64),
    Str(&'a [u8]),
}

/// A dictionary inside a document.
#[derive(Clone, Copy)]
pub struct DictRef<'a> {
    doc: Doc<'a>,
    first: usize,
    count: usize,
    wide: bool,
}

impl Debug for DictRef<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "Dict[{}]", self.count)
    }
}

impl<'a> DictRef<'a> {
    fn slot(&self, index: usize) -> Option<usize> {
        self.doc.resolve(self.first + index * if self.wide { 4 } else { 2 }, self.wide)
    }

    fn key_at(&self, entry: usize) -> Option<Key<'a>> {
        match self.doc.raw(self.slot(2 * entry)?)?.0 {
            Raw::Int(key) => Some(Key::Int(key)),
            Raw::Str(key) => Some(Key::Str(key)),
            _ => None,
        }
    }

    fn value_at(&self, entry: usize) -> Option<ValueRef<'a>> {
        Some(ValueRef { doc: self.doc, pos: self.slot(2 * entry + 1)? })
    }

    /// Whether [`DictRef::iter`], which gave `entries` entries, gave them all: it leaves out
    /// an entry whose key cannot be read, which a copy of the dictionary must not lose
    /// without a word.
    pub(crate) fn gave_all(&self, entries: usize) -> bool {
        if !self.inherits() {
            return entries == self.count;
        }
        // Entries hide one another along the dictionaries inherited from, so each of
        // these is asked for its own.
        let mut dict = Some(*self);
        while let Some(current) = dict {
            let mut own = usize::from(current.inherits())..current.count;
            if !own.all(|entry| current.entry(entry).is_some()) {
                return false;
            }
            dict = current.parent();
        }
        true
    }

    /// Whether the first entry points at a dictionary this one inherits from.
    fn inherits(&self) -> bool {
        self.count > 0 && self.key_at(0) == Some(Key::Int(PARENT_KEY))
    }

    /// The dictionary this one inherits from. It lies before this one, as what a document
    /// is appended to does: in bytes nobody checked a pointer may lead from a dictionary
    /// to itself, and the dictionaries inherited from would then never end.
    fn parent(&self) -> Option<DictRef<'a>> {
        if !self.inherits() {
            return None;
        }
        self.value_at(0)?.as_dict().filter(|parent| parent.first < self.first)
    }

    /// The entry of `key` among the entries of this dictionary alone, by binary search.
    fn find_entry(&self, key: Key<'_>) -> Option<usize> {
        let (mut low, mut high) = (usize::from(self.inherits()), self.count);
        while low < high {
            let middle = low + (high - low) / 2;
            match self.key_at(middle)?.cmp(&key) {
                std::cmp::Ordering::Less => low = middle + 1,
                std::cmp::Ordering::Greater => high = middle,
                std::cmp::Ordering::Equal => return Some(middle),
            }
        }
        None
    }

    fn find(&self, key: Key<'_>) -> Option<ValueRef<'a>> {
        self.value_at(self.find_entry(key)?)
    }

    /// What the dictionaries inherited from give for the key of an entry of this one: the
    /// value that entry hides.
    fn hidden(&self, entry: usize) -> Option<ValueRef<'a>> {
        self.parent()?.get(self.entry(entry)?.0)
    }

    /// How many dictionaries this one inherits through.
    pub(crate) fn inherit_depth(&self) -> usize {
        std::iter::successors(self.parent(), DictRef::parent).count()
    }

    /// Where this dictionary keeps the string of `key`, when it holds the key as a string.
    pub(crate) fn key_position(&self, key: &str) -> Option<usize> {
        self.slot(2 * self.find_entry(Key::Str(key.as_bytes()))?)
    }

    /// The value of `key`, in logarithmic time.
    pub fn get(&self, key: &str) -> Option<ValueRef<'a>> {
        // A key the shared keys hold is stored as its number. A dictionary written before the
        // key came into the table has it as a string, so each dictionary on the way is
        // asked for both, the nearest one that has either deciding.
        let shared = self.doc.shared.and_then(|shared| shared.encode(key));
        let number = shared.map(|number| Key::Int(i64::from(number)));
        let text = Key::Str(key.as_bytes());
        let mut dict = *self;
        loop {
            let found = number.and_then(|number| dict.find(number)).or_else(|| dict.find(text));
            if let Some(value) = found {
                let deleted = dict.inherits() && matches!(value.item(), Item::Undefined);
                return (!deleted).then_some(value);
            }
            // A parent lies before its child, so the chain ends.
            dict = dict.parent()?;
        }
    }

    fn entry(&self, entry: usize) -> Option<(&'a str, ValueRef<'a>)> {
        let key = match self.key_at(entry)? {
            Key::Str(key) => std::str::from_utf8(key).ok()?,
            Key::Int(key) => self.doc.shared?.decode(u16::try_from(key).ok()?)?,
        };
        Some((key, self.value_at(entry)?))
    }

    /// The number of entries, those inherited included.
    pub fn len(&self) -> usize {
        if self.inherits() { self.iter().count() } else { self.count }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The entries. A dictionary that inherits gives its effective contents, sorted by key;
    /// any other gives its entries in stored order, without allocating. An entry whose key
    /// cannot be read (a shared key the document has no name for) is left out.
    pub fn iter(&self) -> DictIter<'a> {
        if !self.inherits() {
            return DictIter::Own { dict: *self, next: 0 };
        }
        let mut chain = vec![*self];
        while let Some(parent) = chain.last().and_then(DictRef::parent) {
            chain.push(parent);
        }
        let mut merged = BTreeMap::new();
        for dict in chain.iter().rev() {
            let inherits = dict.inherits();
            for (key, value) in (usize::from(inherits)..dict.count).filter_map(|i| dict.entry(i)) {
                if inherits && matches!(value.item(), Item::Undefined) {
                    merged.remove(key);
                } else {
                    merged.insert(key, value);
                }
            }
        }
        DictIter::Merged(merged.into_iter())
    }
}

/// The entries of a dictionary; see [`DictRef::iter`].
pub enum DictIter<'a> {
    #[doc(hidden)]
    Own { dict: DictRef<'a>, next: usize },
    #[doc(hidden)]
    Merged(std::collections::btree_map::IntoIter<&'a str, ValueRef<'a>>),
}

impl<'a> Iterator for DictIter<'a> {
    type Item = (&'a str, ValueRef<'a>);

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Merged(entries) => entries.next(),
            Self::Own { dict, next } => {
                while *next < dict.count {
                    *next += 1;
                    if let Some(entry) = dict.entry(*next - 1) {
                        return Some(entry);
                    }
                }
                None
            }
        }
    }
}
