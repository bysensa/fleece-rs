//! Writing a document, bottom up: a collection's items are written before the collection.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

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

/// The hashing of the table of short strings. A string of up to 15 bytes and its length
/// are one 128-bit number, which is hashed by one multiplication with two numbers picked
/// at random for each encoder, so that strings chosen by someone else cannot be made to
/// fall into one bucket.
#[derive(Clone, Copy)]
struct Seeds(u64, u64);

impl Seeds {
    fn random() -> Self {
        let state = RandomState::new();
        Self(state.hash_one(0u8), state.hash_one(1u8))
    }

    fn fold(self, low: u64, high: u64) -> u64 {
        let product = u128::from(low ^ self.0) * u128::from(high ^ self.1);
        product as u64 ^ (product >> 64) as u64
    }
}

impl BuildHasher for Seeds {
    type Hasher = Fold;

    fn build_hasher(&self) -> Fold {
        Fold { seeds: *self, hash: 0 }
    }
}

struct Fold {
    seeds: Seeds,
    hash: u64,
}

impl Hasher for Fold {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.hash = self.seeds.fold(u64::from_le_bytes(word), self.hash);
        }
    }

    fn write_u128(&mut self, key: u128) {
        self.hash = self.seeds.fold(key as u64, (key >> 64) as u64 ^ self.hash);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}

/// A string of 2 to 15 bytes as one number: its bytes, then its length.
fn string_key(bytes: &[u8]) -> u128 {
    let mut key = [0; 16];
    key[..bytes.len()].copy_from_slice(bytes);
    key[15] = bytes.len() as u8;
    u128::from_le_bytes(key)
}

/// A collection that is open. Its slots and keys are the last ones of the encoder's, from
/// these indexes on.
struct Frame {
    dict: bool,
    /// The dictionary of the base document this one inherits from: only the entries that
    /// differ are written.
    parent: Option<usize>,
    /// The kind of the extended value this collection is the payload of.
    ext: Option<u8>,
    first_slot: usize,
    first_key: usize,
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
    /// The slots of every open collection, those of the innermost one last.
    slots: Vec<Slot>,
    /// The keys of every open dictionary, in the same order.
    keys: Vec<SortKey>,
    /// The order of the entries of the dictionary being ended, kept between dictionaries.
    order: Vec<usize>,
    /// The slots of the dictionary being ended in that order, kept as well.
    ordered: Vec<Slot>,
    /// The positions of the short strings written so far, by [`string_key`].
    strings: HashMap<u128, usize, Seeds>,
    shared: Shared<'k>,
    root: Option<Slot>,
    pending_ext: Option<u8>,
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
            slots: Vec::new(),
            keys: Vec::new(),
            order: Vec::new(),
            ordered: Vec::new(),
            strings: HashMap::with_hasher(Seeds::random()),
            shared: Shared::None,
            root: None,
            pending_ext: None,
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
                self.slots.push(slot);
                frame.expects_key = frame.dict;
            }
            None => self.root = Some(slot),
        }
    }

    /// Writes a value that is not a collection, after the header of the extended value it is
    /// the payload of, when there is one. `put` appends the bytes of the whole value.
    fn scalar(&mut self, put: impl FnOnce(&mut Vec<u8>)) -> Result<(), EncodeError> {
        self.check_value()?;
        let start = self.out.len();
        if let Some(kind) = self.pending_ext.take() {
            self.out.extend_from_slice(&[SPECIAL_EXT, kind]);
        }
        put(&mut self.out);
        // A value of 2 bytes lives in the slot that holds it, not in the output.
        let slot = match self.out[start..] {
            [a, b] => {
                self.out.truncate(start);
                Slot::Inline([a, b])
            }
            _ => Slot::Pointer(self.base_len + start),
        };
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

    /// Binary data of `len` bytes that `fill` appends, written without a copy of them.
    pub(crate) fn write_data_from(
        &mut self,
        len: usize,
        fill: impl FnOnce(&mut Vec<u8>),
    ) -> Result<(), EncodeError> {
        self.scalar(|out| {
            put_bytes_header(out, TAG_DATA, len);
            fill(out);
            pad(out);
        })
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

    /// Fleece's `undefined`: the value of a key deleted from a dictionary that inherits.
    pub(crate) fn write_undefined(&mut self) -> Result<(), EncodeError> {
        self.scalar(|out| out.extend_from_slice(&[SPECIAL_UNDEFINED, 0]))
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
        // A short string seen before is at the position the table has for it.
        let next = self.position();
        let position = if SHARED_STRING_LEN.contains(&bytes.len()) {
            *self.strings.entry(string_key(bytes)).or_insert(next)
        } else {
            next
        };
        if position == next {
            put_bytes(&mut self.out, TAG_STRING, bytes);
        }
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
        self.slots.push(slot);
        self.keys.push(sort_key);
        if let Some(frame) = self.stack.last_mut() {
            frame.expects_key = false;
        }
        Ok(())
    }

    fn begin(&mut self, dict: bool, parent: Option<usize>) -> Result<(), EncodeError> {
        self.check_value()?;
        if self.stack.len() >= MAX_DEPTH {
            return Err(EncodeError::TooDeep);
        }
        if self.slots.capacity() == 0 {
            // The first collection: room for a small document at once, where growing from
            // nothing would take each of these through several sizes.
            self.out.reserve(256);
            self.stack.reserve(8);
            self.slots.reserve(32);
            self.keys.reserve(16);
            self.strings.reserve(16);
        }
        self.stack.push(Frame {
            dict,
            parent,
            ext: self.pending_ext.take(),
            first_slot: self.slots.len(),
            first_key: self.keys.len(),
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
        // The slots and keys of the collection are lent out of the encoder while it is
        // written, and given back with the collection taken off their end.
        let slots = std::mem::take(&mut self.slots);
        let keys = std::mem::take(&mut self.keys);
        let slot = self.collection(&frame, &slots[frame.first_slot..], &keys[frame.first_key..]);
        self.slots = slots;
        self.slots.truncate(frame.first_slot);
        self.keys = keys;
        self.keys.truncate(frame.first_key);
        self.push(slot?);
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

    /// Fills `self.ordered` with the slots of a dictionary as it stores them: the entry of
    /// the parent, then the entries by key. Refuses a key that is there twice.
    fn order_entries(
        &mut self,
        parent: Option<usize>,
        slots: &[Slot],
        keys: &[SortKey],
    ) -> Result<(), EncodeError> {
        let mut order = std::mem::take(&mut self.order);
        order.clear();
        order.extend(0..keys.len());
        order.sort_unstable_by(|&a, &b| self.compare_keys(&keys[a], &keys[b]));
        let twice = order
            .windows(2)
            .find(|pair| self.compare_keys(&keys[pair[0]], &keys[pair[1]]) == Ordering::Equal);
        let result = match twice {
            Some(pair) => Err(EncodeError::DuplicateKey(match keys[pair[0]] {
                SortKey::Shared(number) => self.shared.key(number).unwrap_or_default().to_owned(),
                ref key => String::from_utf8_lossy(self.key_bytes(key)).into_owned(),
            })),
            None => Ok(()),
        };
        self.ordered.clear();
        // The entry of the parent has the lowest key there is, so it comes first.
        let parent = parent.map(|parent| [Slot::Inline([0x08, 0x00]), Slot::Pointer(parent)]);
        let entries = order.iter().map(|&i| [slots[2 * i], slots[2 * i + 1]]);
        self.ordered.extend(parent.into_iter().chain(entries).flatten());
        self.order = order;
        result
    }

    /// Writes a collection whose items are written, and gives the slot that stands for it.
    fn collection(
        &mut self,
        frame: &Frame,
        slots: &[Slot],
        keys: &[SortKey],
    ) -> Result<Slot, EncodeError> {
        // Keys that came in order and once each, as those of a sorted map do, stay as they
        // are; anything else is put in order first.
        let in_order = keys.windows(2).all(|pair| self.compare_keys(&pair[0], &pair[1]).is_lt());
        if !frame.dict || (in_order && frame.parent.is_none()) {
            return self.write_collection(frame, slots);
        }
        self.order_entries(frame.parent, slots, keys)?;
        let ordered = std::mem::take(&mut self.ordered);
        let slot = self.write_collection(frame, &ordered);
        self.ordered = ordered;
        slot
    }

    fn write_collection(&mut self, frame: &Frame, slots: &[Slot]) -> Result<Slot, EncodeError> {
        let count = if frame.dict { slots.len() / 2 } else { slots.len() };
        let tag = if frame.dict { TAG_DICT } else { TAG_ARRAY };
        if count == 0 && frame.ext.is_none() {
            return Ok(Slot::Inline([tag << 4, 0]));
        }
        let start = self.out.len();
        let position = self.position();
        // The collection is narrow when every pointer of it reaches its value in 2 bytes,
        // which nearly all do: it is written so, and written again wide when one does not.
        self.out.reserve(16 + slots.len() * 2);
        self.collection_header(frame.ext, tag, false, count);
        let first = self.position();
        let narrow = slots.iter().enumerate().all(|(i, slot)| match *slot {
            Slot::Inline(bytes) => {
                self.out.extend_from_slice(&bytes);
                true
            }
            Slot::Pointer(target) => {
                let offset = first + 2 * i - target;
                let external = self.external && target < self.base_len;
                let flags = 0x8000 | if external { 0x4000 } else { 0 };
                self.out.extend_from_slice(&((offset / 2) as u16 | flags).to_be_bytes());
                offset <= MAX_NARROW_OFFSET
            }
        });
        if narrow {
            return Ok(Slot::Pointer(position));
        }

        self.out.truncate(start);
        let pointers = slots.iter().enumerate().filter_map(|(i, slot)| match slot {
            Slot::Pointer(target) => Some(first + 4 * i - target),
            Slot::Inline(_) => None,
        });
        if pointers.max().unwrap_or(0) > MAX_WIDE_OFFSET {
            return Err(EncodeError::TooLarge);
        }
        self.out.reserve(16 + slots.len() * 4);
        self.collection_header(frame.ext, tag, true, count);
        for slot in slots {
            match *slot {
                Slot::Inline(bytes) => self.out.extend_from_slice(&[bytes[0], bytes[1], 0, 0]),
                Slot::Pointer(target) => self.pointer(target, true),
            }
        }
        Ok(Slot::Pointer(position))
    }

    /// Writes what comes before the slots of a collection.
    fn collection_header(&mut self, ext: Option<u8>, tag: u8, wide: bool, count: usize) {
        if let Some(kind) = ext {
            self.out.extend_from_slice(&[SPECIAL_EXT, kind]);
        }
        // A count of 2047 or more is 2047 in the header and the rest in a varint after it.
        let short = count.min(LONG_COUNT);
        let flags = tag << 4 | if wide { 0x08 } else { 0 };
        self.out.extend_from_slice(&[flags | (short >> 8) as u8, short as u8]);
        if let Some(rest) = count.checked_sub(LONG_COUNT) {
            put_varint(&mut self.out, rest as u64);
            pad(&mut self.out);
        }
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
