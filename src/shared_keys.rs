//! Dictionary keys shared by many documents: each key is stored as a small number.

use std::collections::HashMap;

use crate::encoder::Encoder;
use crate::error::{DecodeError, EncodeError};
use crate::reader::Doc;

/// A table that gives short dictionary keys numbers, in the order they were first seen.
///
/// Every document written with the table has to be read with it, or with a later state of
/// it: a table only grows. Keeping the table is up to the caller; [`SharedKeys::to_state`] and
/// [`SharedKeys::from_state`] turn it into bytes and back.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SharedKeys {
    keys: Vec<Box<str>>,
    numbers: HashMap<Box<str>, u16>,
}

impl SharedKeys {
    /// The most keys a table holds: the numbers have to fit a 2-byte value.
    pub const MAX_KEYS: usize = 2048;
    /// The longest key a table takes, in bytes.
    pub const MAX_KEY_LEN: usize = 16;

    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Whether a table may hold `key`: 1 to 16 ASCII letters, digits, `_` or `-`.
    pub fn is_eligible(key: &str) -> bool {
        (1..=Self::MAX_KEY_LEN).contains(&key.len())
            && key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    }

    /// The number of `key`, when the table holds it.
    pub fn encode(&self, key: &str) -> Option<u16> {
        self.numbers.get(key).copied()
    }

    /// The number of `key`, which is added to the table when it is eligible and there is room.
    pub fn encode_and_add(&mut self, key: &str) -> Option<u16> {
        if let Some(number) = self.encode(key) {
            return Some(number);
        }
        if !Self::is_eligible(key) || self.keys.len() >= Self::MAX_KEYS {
            return None;
        }
        let number = self.keys.len() as u16;
        self.keys.push(key.into());
        self.numbers.insert(key.into(), number);
        Some(number)
    }

    /// Forgets the keys added since the table had `len` keys.
    ///
    /// A table grows while a document is written, and the document and the table have to
    /// reach the store together. When the write of the document fails, or its transaction is
    /// tried again, the keys it added are taken back with the length the table had before,
    /// so that their numbers are not given to other keys by someone else meanwhile.
    pub fn truncate(&mut self, len: usize) {
        for key in self.keys.drain(len.min(self.keys.len())..) {
            self.numbers.remove(&key);
        }
    }

    /// The key with this number.
    pub fn decode(&self, number: u16) -> Option<&str> {
        self.keys.get(usize::from(number)).map(|key| &**key)
    }

    /// The keys in the order of their numbers.
    pub fn keys(&self) -> impl ExactSizeIterator<Item = &str> {
        self.keys.iter().map(|key| &**key)
    }

    /// The table as a Fleece document: an array of the keys.
    pub fn to_state(&self) -> Result<Vec<u8>, EncodeError> {
        let mut encoder = Encoder::new();
        encoder.begin_array()?;
        for key in &self.keys {
            encoder.write_str(key)?;
        }
        encoder.end_array()?;
        encoder.finish()
    }

    /// The table [`SharedKeys::to_state`] wrote.
    pub fn from_state(state: &[u8]) -> Result<Self, DecodeError> {
        let root = Doc::new(state)?.root();
        let malformed = DecodeError::Malformed { pos: root.position() };
        let mut shared = Self::new();
        for key in root.as_array().ok_or(malformed.clone())?.iter() {
            let key = key.as_str().ok_or(malformed.clone())?;
            if shared.encode(key).is_some() || shared.encode_and_add(key).is_none() {
                return Err(malformed);
            }
        }
        Ok(shared)
    }
}
