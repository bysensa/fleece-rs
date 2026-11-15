//! The hashing of the tables the encoder and the check of a document keep.
//!
//! Their keys are numbers (a short string as one 128-bit number, a position in a document),
//! which one multiplication hashes well enough, where the default SipHash of the standard
//! library took a sixth of the time of encoding. The two numbers multiplied with are picked
//! at random for each table, so that keys chosen by someone else, which the positions in a
//! document not trusted are, cannot be made to fall into one bucket.

use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};

#[derive(Clone, Copy)]
pub(crate) struct Seeds(u64, u64);

impl Seeds {
    pub(crate) fn random() -> Self {
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

pub(crate) struct Fold {
    seeds: Seeds,
    hash: u64,
}

impl Hasher for Fold {
    fn write(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8) {
            let mut word = [0; 8];
            word[..chunk.len()].copy_from_slice(chunk);
            self.write_u64(u64::from_le_bytes(word));
        }
    }

    fn write_u64(&mut self, key: u64) {
        self.hash = self.seeds.fold(key, self.hash);
    }

    fn write_usize(&mut self, key: usize) {
        self.write_u64(key as u64);
    }

    fn write_u128(&mut self, key: u128) {
        self.hash = self.seeds.fold(key as u64, (key >> 64) as u64 ^ self.hash);
    }

    fn finish(&self) -> u64 {
        self.hash
    }
}
