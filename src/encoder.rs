//! Writing a document, bottom up: a collection's items are written before the collection.

use std::cmp::Ordering;
use std::collections::HashMap;

use crate::error::EncodeError;
use crate::format::*;
use crate::reader::{Item, ValueRef};
use crate::shared_keys::SharedKeys;

/// Strings of these lengths are written once and pointed at afterwards.
const SHARED_STRING_LEN: std::ops::RangeInclusive<usize> = 2..=15;

/// What a collection keeps for one item: the 2 bytes of a value that fits them, or the
/// position of a value written earlier.
#[derive(Clone, Copy)]
enum Slot {
    Inline([u8; 2]),
    Pointer(usize),
}

/// A dictionary key, kept to sort the entries by.
enum SortKey {
    Shared(u16),
    /// A string of at most one byte, which lives in its slot.
    Tiny([u8; 1], usize),
    /// A longer string: where its bytes start in the output, and how many they are.
    Written(usize, usize),
    /// A string of the base document, which the output does not hold.
    InBase(Box<[u8]>),
}

/// The shared keys an encoder writes dictionary keys with.
enum Shared<'k> {
    None,
    /// Keys that are not in the table yet are added to it.
    Growing(&'k mut SharedKeys),
    /// Only the keys the table holds are written as numbers.
    Fixed(&'k SharedKeys),
}

impl Shared<'_> {
    fn number(&mut self, key: &str) -> Option<u16> {
        match self {
            Self::None => None,
            Self::Growing(shared) => shared.encode_and_add(key),
            Self::Fixed(shared) => shared.encode(key),
        }
    }

    fn key(&self, number: u16) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Growing(shared) => shared.decode(number),
            Self::Fixed(shared) => shared.decode(number),
        }
    }
}

struct Frame {
    dict: bool,
    /// The dictionary of the base document this one inherits from: only the entries that
    /// differ are written.
    parent: Option<usize>,
    /// The kind of the extended value this collection is the payload of.
    ext: Option<u8>,
    slots: Vec<Slot>,
    keys: Vec<SortKey>,
    /// Whether a dictionary takes a key next.
    expects_key: bool,
}

/// Builds one document from a sequence of calls: one value at the top level, a collection
/// being `begin_*`, its items, `end_*`.
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// let mut encoder = fleece::Encoder::new();
/// encoder.begin_dict()?;
/// encoder.write_key("foo")?;
/// encoder.write_int(123)?;
/// encoder.end_dict()?;
/// let bytes = encoder.finish()?;
/// let root = fleece::Doc::new(&bytes)?.root();
/// assert_eq!(root.as_dict().and_then(|dict| dict.get("foo")?.as_i64()), Some(123));
/// # Ok(())
/// # }
/// ```
pub struct Encoder<'k> {
    out: Vec<u8>,
    /// The length of the base document: the position of the first byte written.
    base_len: usize,
    /// Whether pointers into the base are marked as leaving the document.
    external: bool,
    stack: Vec<Frame>,
    /// The positions of the short strings written so far.
    strings: HashMap<Box<[u8]>, usize>,
    shared: Shared<'k>,
    root: Option<Slot>,
    pending_ext: Option<u8>,
    scratch: Vec<u8>,
}

impl Default for Encoder<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'k> Encoder<'k> {
    pub fn new() -> Self {
        Self {
            out: Vec::new(),
            base_len: 0,
            external: false,
            stack: Vec::new(),
            strings: HashMap::new(),
            shared: Shared::None,
            root: None,
            pending_ext: None,
            scratch: Vec::new(),
        }
    }

    /// An encoder of a delta over `base`: values of the base given to
    /// [`Encoder::write_ref`] are pointed at, not written again.
    ///
    /// With `external` false the result is meant to be appended to the base, the two then
    /// forming one document. With `external` true the pointers into the base are marked, and
    /// the result is read apart from it with [`crate::Doc::with_base`].
    pub fn with_base(mut self, base: &[u8], external: bool) -> Result<Self, EncodeError> {
        if base.len() & 1 == 1 {
            return Err(EncodeError::BadBase);
        }
        self.base_len = base.len();
        self.external = external;
        Ok(self)
    }

    /// An encoder that writes eligible dictionary keys as numbers of `shared`, adding the
    /// keys it does not hold yet.
    pub fn with_shared_keys(mut self, shared: &'k mut SharedKeys) -> Self {
        self.shared = Shared::Growing(shared);
        self
    }

    /// An encoder that writes the keys `shared` holds as their numbers and adds none: for
    /// writing anew what was written with the table, which then needs no change.
    pub fn with_fixed_shared_keys(mut self, shared: &'k SharedKeys) -> Self {
        self.shared = Shared::Fixed(shared);
        self
    }

    fn position(&self) -> usize {
        self.base_len + self.out.len()
    }

    /// Writes the bytes of a whole value and gives the slot that stands for it.
    fn place(&mut self, bytes: &[u8]) -> Slot {
        if let &[a, b] = bytes {
            return Slot::Inline([a, b]);
        }
        let position = self.position();
        self.out.extend_from_slice(bytes);
        Slot::Pointer(position)
    }

    /// Refuses a value where one is not taken.
    fn check_value(&self) -> Result<(), EncodeError> {
        match self.stack.last() {
            Some(frame) if frame.dict && frame.expects_key => Err(EncodeError::ExpectedKey),
            None if self.root.is_some() => Err(EncodeError::MultipleRoots),
            _ => Ok(()),
        }
    }

    fn push(&mut self, slot: Slot) {
        match self.stack.last_mut() {
            Some(frame) => {
                frame.slots.push(slot);
                frame.expects_key = frame.dict;
            }
            None => self.root = Some(slot),
        }
    }

    /// Writes a value that is not a collection, after the header of the extended value it is
    /// the payload of, when there is one.
    fn scalar(&mut self, put: impl FnOnce(&mut Vec<u8>)) -> Result<(), EncodeError> {
        self.check_value()?;
        let mut bytes = std::mem::take(&mut self.scratch);
        bytes.clear();
        if let Some(kind) = self.pending_ext.take() {
            bytes.extend_from_slice(&[SPECIAL_EXT, kind]);
        }
        put(&mut bytes);
        let slot = self.place(&bytes);
        self.scratch = bytes;
        self.push(slot);
        Ok(())
    }

    pub fn write_null(&mut self) -> Result<(), EncodeError> {
        self.scalar(|out| out.extend_from_slice(&[SPECIAL_NULL, 0]))
    }

    pub fn write_bool(&mut self, value: bool) -> Result<(), EncodeError> {
        let byte = if value { SPECIAL_TRUE } else { SPECIAL_FALSE };
        self.scalar(|out| out.extend_from_slice(&[byte, 0]))
    }

    pub fn write_int(&mut self, value: i64) -> Result<(), EncodeError> {
        self.scalar(|out| put_int(out, value))
    }

    /// An integer marked as unsigned, the form the whole range of `u64` takes.
    pub fn write_uint(&mut self, value: u64) -> Result<(), EncodeError> {
        self.scalar(|out| put_uint(out, value))
    }

    pub fn write_f32(&mut self, value: f32) -> Result<(), EncodeError> {
        self.scalar(|out| put_f32(out, value))
    }

    pub fn write_f64(&mut self, value: f64) -> Result<(), EncodeError> {
        self.scalar(|out| put_f64(out, value))
    }

    pub fn write_data(&mut self, value: &[u8]) -> Result<(), EncodeError> {
        self.scalar(|out| put_bytes(out, TAG_DATA, value))
    }

    pub fn write_str(&mut self, value: &str) -> Result<(), EncodeError> {
        if self.pending_ext.is_some() {
            return self.scalar(|out| put_bytes(out, TAG_STRING, value.as_bytes()));
        }
        self.check_value()?;
        let (slot, _) = self.string(value.as_bytes());
        self.push(slot);
        Ok(())
    }

    /// The extended value without a payload: SurrealDB's `NONE`.
    pub fn write_none(&mut self) -> Result<(), EncodeError> {
        if self.pending_ext.is_some() {
            return Err(EncodeError::ExtPayload);
        }
        self.scalar(|out| out.extend_from_slice(&[SPECIAL_EXT, ext::NONE]))
    }

    /// Makes the next value written, whether a scalar or a whole collection, the payload of
    /// an extended value of `kind` (see [`crate::ext`]).
    pub fn begin_ext(&mut self, kind: u8) -> Result<(), EncodeError> {
        if kind == ext::NONE || kind > ext::MAX {
            return Err(EncodeError::BadExtKind(kind));
        }
        if self.pending_ext.is_some() {
            return Err(EncodeError::ExtPayload);
        }
        self.check_value()?;
        self.pending_ext = Some(kind);
        Ok(())
    }

    /// Writes a string, or finds the one already written, and gives its slot and sort key.
    fn string(&mut self, bytes: &[u8]) -> (Slot, SortKey) {
        if bytes.len() <= 1 {
            let byte = bytes.first().copied().unwrap_or(0);
            let slot = Slot::Inline([TAG_STRING << 4 | bytes.len() as u8, byte]);
            return (slot, SortKey::Tiny([byte], bytes.len()));
        }
        let shared = SHARED_STRING_LEN.contains(&bytes.len());
        let position = match self.strings.get(bytes) {
            Some(&position) if shared => position,
            _ => {
                let position = self.position();
                put_bytes(&mut self.out, TAG_STRING, bytes);
                if shared {
                    self.strings.insert(bytes.into(), position);
                }
                position
            }
        };
        let start = position - self.base_len + bytes_offset(bytes.len());
        (Slot::Pointer(position), SortKey::Written(start, bytes.len()))
    }

    /// Writes the key of the next entry of the open dictionary.
    pub fn write_key(&mut self, key: &str) -> Result<(), EncodeError> {
        self.write_key_at(key, None)
    }

    /// Writes a key whose string the base document holds at `in_base`, when it holds it.
    pub(crate) fn write_key_at(
        &mut self,
        key: &str,
        in_base: Option<usize>,
    ) -> Result<(), EncodeError> {
        let takes_key = self.stack.last().is_some_and(|frame| frame.dict && frame.expects_key);
        if !takes_key || self.pending_ext.is_some() {
            return Err(EncodeError::UnexpectedKey);
        }
        let (slot, sort_key) = match self.shared.number(key) {
            Some(number) => {
                let mut bytes = [0; 2];
                bytes.copy_from_slice(&(number & 0x0FFF).to_be_bytes());
                (Slot::Inline(bytes), SortKey::Shared(number))
            }
            None => match in_base.filter(|position| key.len() > 1 && *position < self.base_len) {
                Some(position) => (Slot::Pointer(position), SortKey::InBase(key.as_bytes().into())),
                None => self.string(key.as_bytes()),
            },
        };
        if let Some(frame) = self.stack.last_mut() {
            frame.slots.push(slot);
            frame.keys.push(sort_key);
            frame.expects_key = false;
        }
        Ok(())
    }

    fn begin(&mut self, dict: bool, parent: Option<usize>) -> Result<(), EncodeError> {
        self.check_value()?;
        if self.stack.len() >= MAX_DEPTH {
            return Err(EncodeError::TooDeep);
        }
        self.stack.push(Frame {
            dict,
            parent,
            ext: self.pending_ext.take(),
            slots: Vec::new(),
            keys: Vec::new(),
            expects_key: dict,
        });
        Ok(())
    }

    pub fn begin_array(&mut self) -> Result<(), EncodeError> {
        self.begin(false, None)
    }

    pub fn begin_dict(&mut self) -> Result<(), EncodeError> {
        self.begin(true, None)
    }

    /// Begins a dictionary that inherits from the dictionary of the base document at
    /// `parent`: it has every entry of that one, but for the entries written into it, an
    /// entry whose value is `undefined` being a deletion.
    pub(crate) fn begin_dict_over(&mut self, parent: usize) -> Result<(), EncodeError> {
        if parent >= self.base_len {
            return Err(EncodeError::NotInBase);
        }
        self.begin(true, Some(parent))
    }

    /// Fleece's `undefined`: the value of a key deleted from a dictionary that inherits.
    pub(crate) fn write_undefined(&mut self) -> Result<(), EncodeError> {
        self.scalar(|out| out.extend_from_slice(&[SPECIAL_UNDEFINED, 0]))
    }

    pub fn end_array(&mut self) -> Result<(), EncodeError> {
        self.end(false)
    }

    pub fn end_dict(&mut self) -> Result<(), EncodeError> {
        self.end(true)
    }

    fn end(&mut self, dict: bool) -> Result<(), EncodeError> {
        let matches = self.stack.last().is_some_and(|frame| {
            frame.dict == dict && frame.expects_key == dict && self.pending_ext.is_none()
        });
        // A refused call leaves the open collection as it was.
        let Some(frame) = self.stack.pop_if(|_| matches) else {
            return Err(EncodeError::Unbalanced);
        };
        let slot = self.collection(frame)?;
        self.push(slot);
        Ok(())
    }

    fn key_bytes<'s>(&'s self, key: &'s SortKey) -> &'s [u8] {
        match key {
            SortKey::Shared(_) => &[],
            SortKey::Tiny(byte, len) => &byte[..*len],
            SortKey::Written(start, len) => &self.out[*start..*start + *len],
            SortKey::InBase(bytes) => bytes,
        }
    }

    /// The order of keys in a dictionary: shared keys by number, then strings by bytes.
    fn compare_keys(&self, a: &SortKey, b: &SortKey) -> Ordering {
        match (a, b) {
            (SortKey::Shared(a), SortKey::Shared(b)) => a.cmp(b),
            (SortKey::Shared(_), _) => Ordering::Less,
            (_, SortKey::Shared(_)) => Ordering::Greater,
            _ => self.key_bytes(a).cmp(self.key_bytes(b)),
        }
    }

    fn collection(&mut self, frame: Frame) -> Result<Slot, EncodeError> {
        let Frame { dict, parent, ext, mut slots, keys, .. } = frame;
        let mut count = if dict { keys.len() } else { slots.len() };
        if dict {
            let mut order: Vec<usize> = (0..count).collect();
            order.sort_by(|&a, &b| self.compare_keys(&keys[a], &keys[b]));
            for pair in order.windows(2) {
                if self.compare_keys(&keys[pair[0]], &keys[pair[1]]) == Ordering::Equal {
                    let key = match keys[pair[0]] {
                        SortKey::Shared(number) => {
                            self.shared.key(number).unwrap_or_default().to_owned()
                        }
                        ref key => String::from_utf8_lossy(self.key_bytes(key)).into_owned(),
                    };
                    return Err(EncodeError::DuplicateKey(key));
                }
            }
            // The entry of the parent has the lowest key there is, so it comes first.
            let parent = parent.map(|parent| [Slot::Inline([0x08, 0x00]), Slot::Pointer(parent)]);
            let entries = order.iter().map(|&i| [slots[2 * i], slots[2 * i + 1]]);
            slots = parent.into_iter().chain(entries).flatten().collect();
            count = slots.len() / 2;
        }
        let tag = if dict { TAG_DICT } else { TAG_ARRAY };
        if count == 0 && ext.is_none() {
            return Ok(Slot::Inline([tag << 4, 0]));
        }

        let position = self.position();
        let mut header = Vec::with_capacity(8);
        if let Some(kind) = ext {
            header.extend_from_slice(&[SPECIAL_EXT, kind]);
        }
        let short = count.min(LONG_COUNT);
        let tag_at = header.len();
        header.extend_from_slice(&[tag << 4 | (short >> 8) as u8, short as u8]);
        if count >= LONG_COUNT {
            put_varint(&mut header, (count - LONG_COUNT) as u64);
            if header.len() & 1 == 1 {
                header.push(0);
            }
        }
        // The collection is narrow when every pointer of it reaches its value in 2 bytes.
        let first = position + header.len();
        let farthest = |width: usize| {
            let offsets = slots.iter().enumerate().filter_map(|(i, slot)| match slot {
                Slot::Pointer(target) => Some(first + i * width - target),
                Slot::Inline(_) => None,
            });
            offsets.max().unwrap_or(0)
        };
        let wide = farthest(2) > MAX_NARROW_OFFSET;
        if wide {
            if farthest(4) > MAX_WIDE_OFFSET {
                return Err(EncodeError::TooLarge);
            }
            header[tag_at] |= 0x08;
        }
        self.out.extend_from_slice(&header);
        for slot in slots {
            match slot {
                Slot::Inline(bytes) => {
                    self.out.extend_from_slice(&bytes);
                    if wide {
                        self.out.extend_from_slice(&[0, 0]);
                    }
                }
                Slot::Pointer(target) => self.pointer(target, wide),
            }
        }
        Ok(Slot::Pointer(position))
    }

    /// Writes a pointer from the current position back to `target`.
    fn pointer(&mut self, target: usize, wide: bool) {
        let units = (self.position() - target) / 2;
        let external = self.external && target < self.base_len;
        if wide {
            let flags = 0x8000_0000 | if external { 0x4000_0000 } else { 0 };
            self.out.extend_from_slice(&(units as u32 | flags).to_be_bytes());
        } else {
            let flags = 0x8000 | if external { 0x4000 } else { 0 };
            self.out.extend_from_slice(&(units as u16 | flags).to_be_bytes());
        }
    }

    /// Writes a value of the base document as a pointer to it, or as its 2 bytes when it is
    /// that small. The value has to come from the document given to [`Encoder::with_base`].
    pub fn write_ref(&mut self, value: ValueRef<'_>) -> Result<(), EncodeError> {
        if self.pending_ext.is_some() {
            return Err(EncodeError::ExtPayload);
        }
        self.check_value()?;
        let position = value.position();
        let size = value.doc().size_of(position).ok_or(EncodeError::NotInBase)?;
        if position & 1 == 1 || position + size > self.base_len {
            return Err(EncodeError::NotInBase);
        }
        let slot = match value.doc().tail(position) {
            &[a, b, ..] if size == 2 => Slot::Inline([a, b]),
            _ => Slot::Pointer(position),
        };
        self.push(slot);
        Ok(())
    }

    /// Writes a copy of a value of any document, with everything it leads to: strings that
    /// repeat are written once, a dictionary that inherits is written whole, and the keys
    /// are written with the shared keys of this encoder.
    pub fn write_copy(&mut self, value: ValueRef<'_>) -> Result<(), EncodeError> {
        match value.item() {
            Item::String(text) => self.write_str(text),
            Item::Array(array) => {
                self.begin_array()?;
                for item in array.iter() {
                    self.write_copy(item)?;
                }
                self.end_array()
            }
            Item::Dict(dict) => {
                self.begin_dict()?;
                for (key, value) in dict.iter() {
                    self.write_key(key)?;
                    self.write_copy(value)?;
                }
                self.end_dict()
            }
            Item::Ext(kind, Some(payload)) => {
                self.begin_ext(kind)?;
                self.write_copy(payload)
            }
            Item::Invalid => Err(EncodeError::InvalidValue),
            // Every other value is its own bytes, wherever they are.
            _ => {
                let position = value.position();
                let size = value.doc().size_of(position).ok_or(EncodeError::InvalidValue)?;
                let bytes = value.doc().tail(position).get(..size);
                let bytes = bytes.ok_or(EncodeError::InvalidValue)?;
                self.scalar(|out| out.extend_from_slice(bytes))
            }
        }
    }

    /// The document: everything written, ended by the root or a pointer to it.
    pub fn finish(mut self) -> Result<Vec<u8>, EncodeError> {
        if !self.stack.is_empty() || self.pending_ext.is_some() {
            return Err(EncodeError::Incomplete);
        }
        match self.root.ok_or(EncodeError::Incomplete)? {
            Slot::Inline(bytes) => self.out.extend_from_slice(&bytes),
            Slot::Pointer(target) => {
                let offset = self.position() - target;
                if offset > MAX_WIDE_OFFSET {
                    return Err(EncodeError::TooLarge);
                }
                if offset > MAX_NARROW_OFFSET {
                    // The document ends with a narrow pointer, so a root too far back for
                    // one is reached through a wide pointer written just before it.
                    self.pointer(target, true);
                    let wide_pointer = self.position() - 4;
                    self.pointer(wide_pointer, false);
                } else {
                    self.pointer(target, false);
                }
            }
        }
        Ok(self.out)
    }
}
