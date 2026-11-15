//! Taking the dead weight out of a document that grew by appended deltas.
//!
//! Every delta appended to a document leaves the values it replaced in the bytes, where
//! nothing points at them any more, and makes dictionaries inherit through one more level.
//! [`Doc::live_bytes`] says how much of the bytes is still in use, and [`compact`] writes
//! the document anew with nothing else in it.

use std::collections::HashSet;

use crate::encoder::Encoder;
use crate::error::DeltaError;
use crate::format::{MAX_DEPTH, ext};
use crate::reader::{Doc, Raw};

/// The document written anew: only the values its root leads to, each string once,
/// dictionaries that inherit written whole, the base of a delta taken in. The document is
/// checked first. Keys stay numbers of the shared keys the document was given.
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use fleece::{Doc, Value, compact, decode, delta::append_delta, encode};
///
/// let mut bytes = encode(&Value::from("the first version"))?;
/// bytes.extend(append_delta(&bytes, &Value::from("the second version"))?);
/// let doc = Doc::new(&bytes)?;
/// assert!(doc.live_bytes() < bytes.len());
///
/// let compacted = compact(&doc)?;
/// assert_eq!(decode(&compacted)?, Value::from("the second version"));
/// assert_eq!(Doc::new(&compacted)?.live_bytes(), compacted.len());
/// # Ok(())
/// # }
/// ```
pub fn compact(doc: &Doc<'_>) -> Result<Vec<u8>, DeltaError> {
    doc.validate()?;
    let mut encoder = Encoder::new();
    if let Some(shared) = doc.shared() {
        encoder = encoder.with_fixed_shared_keys(shared);
    }
    encoder.write_copy(doc.root())?;
    Ok(encoder.finish()?)
}

impl Doc<'_> {
    /// The bytes of the document and its base that the root leads to, the pointer that ends
    /// the document included. The rest, up to the length of base and document, is what
    /// deltas left behind; [`compact`] drops it.
    pub fn live_bytes(&self) -> usize {
        let mut live = Live { doc: self, seen: HashSet::new(), bytes: 0 };
        let Some(end) = self.end().checked_sub(2) else {
            return 0;
        };
        live.bytes += 2;
        if self.is_pointer(end)
            && let Some((mut target, _)) = self.pointer(end, false)
        {
            if self.is_pointer(target) {
                // The wide pointer a far root is reached through.
                live.bytes += 4;
                target = self.pointer(target, true).map_or(usize::MAX, |pointer| pointer.0);
            }
            live.target(target, 0);
        }
        live.bytes
    }
}

struct Live<'d, 'a> {
    doc: &'d Doc<'a>,
    /// The values reached by a pointer and counted already.
    seen: HashSet<usize>,
    bytes: usize,
}

impl Live<'_, '_> {
    /// Counts the value a pointer leads to, once.
    fn target(&mut self, pos: usize, depth: usize) {
        if self.seen.insert(pos) {
            self.bytes += self.doc.size_of(pos).unwrap_or(0);
            self.inside(pos, depth);
        }
    }

    /// Follows the pointers of a value whose own bytes are counted.
    fn inside(&mut self, pos: usize, depth: usize) {
        if depth > 4 * MAX_DEPTH {
            return;
        }
        match self.doc.raw(pos).map(|raw| raw.0) {
            Some(Raw::Ext(kind)) if kind != ext::NONE => self.inside(pos + 2, depth + 1),
            Some(Raw::Coll { dict, wide, count, first }) => {
                let width = if wide { 4 } else { 2 };
                for i in 0..if dict { 2 * count } else { count } {
                    let addr = first + i * width;
                    if !self.doc.is_pointer(addr) {
                        self.inside(addr, depth + 1);
                    } else if let Some((target, _)) = self.doc.pointer(addr, wide) {
                        self.target(target, depth + 1);
                    }
                }
            }
            _ => {}
        }
    }
}
