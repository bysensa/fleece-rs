//! Vectors of numbers of one type, as embeddings are: stored tightly and compared where
//! they lie.
//!
//! An embedding as an array of numbers would take 6 to 12 bytes a dimension in Fleece (a
//! slot and a float behind it) and a walk over pointers to read. A [`Vector`] is one run of
//! bytes: 4 bytes a dimension as `f32`, 2 as `f16`, 1 as `i8`, 1 bit as a sign. What a search
//! does with embeddings, a distance to a query, [`VectorRef`] computes over those bytes in
//! the document, without a vector being built.
//!
//! # The element types
//!
//! | type | a dimension | what is kept |
//! |---|---|---|
//! | `f32` | 4 bytes | the value |
//! | `f64` | 8 bytes | the value |
//! | `f16` | 2 bytes | the value to about 3 decimal digits, enough for a unit vector |
//! | `i8` | 1 byte | `round(value * 127)`, for values from -1 to 1 |
//! | bits | 1 bit | whether the value is above zero |
//!
//! Every distance is against a query of `f32`, whatever the stored type, and gives what
//! the stored values stand for: an `i8` element counts as its value over 127, a bit as 1 or
//! -1. A search over many vectors compares cheap ones first (bits by [`VectorRef::hamming`],
//! or `i8`) and the full ones only for the best of those.
//!
//! A model trained for it (Matryoshka) gives vectors whose leading dimensions are a
//! shorter embedding. [`VectorRef::prefix`] is those dimensions of a stored vector, with no
//! copy; the cosine of a prefix divides by the lengths of the prefixes, as it must, since a
//! part of a unit vector is not a unit vector.
//!
//! SurrealDB has no such type: there a vector is an array of numbers. This is the one kind
//! of this crate that is not one of its types.

use crate::format::ext;
use crate::reader::{Item, ValueRef};
use crate::value::F16;

/// What one `i8` element is multiplied by to give the value it stands for.
const I8_STEP: f32 = 1.0 / 127.0;

/// The type of the elements of a vector.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Element {
    F32,
    F64,
    F16,
    I8,
    Bit,
}

impl Element {
    fn code(self) -> u8 {
        self as u8
    }

    fn from_code(code: u8) -> Option<Self> {
        [Self::F32, Self::F64, Self::F16, Self::I8, Self::Bit]
            .into_iter()
            .find(|e| e.code() == code)
    }

    /// The bits one dimension takes.
    pub fn bits(self) -> usize {
        match self {
            Self::F32 => 32,
            Self::F64 => 64,
            Self::F16 => 16,
            Self::I8 => 8,
            Self::Bit => 1,
        }
    }

    /// The name of the type, as JSON has it.
    pub fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::F64 => "f64",
            Self::F16 => "f16",
            Self::I8 => "i8",
            Self::Bit => "bits",
        }
    }
}

/// A vector that owns its elements.
#[derive(Clone, Debug, PartialEq)]
pub enum Vector {
    F32(Vec<f32>),
    F64(Vec<f64>),
    F16(Vec<F16>),
    /// Each element is the value times 127.
    I8(Vec<i8>),
    /// A bit for each of `len` dimensions, the first in the lowest bit of the first byte,
    /// the bits past the last dimension zero.
    Bits {
        bytes: Vec<u8>,
        len: usize,
    },
}

impl From<Vec<f32>> for Vector {
    fn from(values: Vec<f32>) -> Self {
        Self::F32(values)
    }
}

impl Vector {
    pub fn element(&self) -> Element {
        match self {
            Self::F32(_) => Element::F32,
            Self::F64(_) => Element::F64,
            Self::F16(_) => Element::F16,
            Self::I8(_) => Element::I8,
            Self::Bits { .. } => Element::Bit,
        }
    }

    /// The number of dimensions.
    pub fn len(&self) -> usize {
        match self {
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
            Self::F16(values) => values.len(),
            Self::I8(values) => values.len(),
            Self::Bits { len, .. } => *len,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The values at half the size: each rounded to the nearest `f16`.
    pub fn quantized_f16(values: &[f32]) -> Self {
        Self::F16(values.iter().map(|value| F16::from_f32(*value)).collect())
    }

    /// The values at a quarter of the size: each times 127, rounded, for values from -1 to
    /// 1 as the elements of a unit vector are. A value outside is brought to the nearest
    /// end.
    pub fn quantized_i8(values: &[f32]) -> Self {
        let element = |value: &f32| (value * 127.0).round().clamp(-127.0, 127.0) as i8;
        Self::I8(values.iter().map(element).collect())
    }

    /// The signs of the values, a bit each: set for a value above zero.
    pub fn quantized_bits(values: &[f32]) -> Self {
        let byte = |chunk: &[f32]| {
            chunk.iter().enumerate().fold(0, |byte, (i, value)| byte | u8::from(*value > 0.0) << i)
        };
        Self::Bits { bytes: values.chunks(8).map(byte).collect(), len: values.len() }
    }

    /// The values the elements stand for.
    pub fn to_f32(&self) -> Vec<f32> {
        match self {
            Self::F32(values) => values.clone(),
            Self::F64(values) => values.iter().map(|value| *value as f32).collect(),
            Self::F16(values) => values.iter().map(|value| value.to_f32()).collect(),
            Self::I8(values) => values.iter().map(|value| f32::from(*value) * I8_STEP).collect(),
            Self::Bits { bytes, len } => {
                (0..*len).map(|i| sign(bytes.get(i / 8).copied().unwrap_or(0), i % 8)).collect()
            }
        }
    }

    /// The bytes of the payload: the element type, for bits the number of unused bits of
    /// the last byte, then the elements.
    pub(crate) fn byte_len(&self) -> usize {
        match self {
            Self::Bits { bytes, .. } => 2 + bytes.len(),
            other => 1 + other.len() * other.element().bits() / 8,
        }
    }

    pub(crate) fn write_bytes(&self, out: &mut Vec<u8>) {
        out.push(self.element().code());
        match self {
            Self::F32(values) => {
                values.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()))
            }
            Self::F64(values) => {
                values.iter().for_each(|v| out.extend_from_slice(&v.to_le_bytes()))
            }
            Self::F16(values) => {
                values.iter().for_each(|v| out.extend_from_slice(&v.0.to_le_bytes()))
            }
            Self::I8(values) => out.extend(values.iter().map(|v| *v as u8)),
            Self::Bits { bytes, len } => {
                let count = len.div_ceil(8);
                out.push((count * 8 - len) as u8);
                // Exactly the bytes the dimensions take, the bits past the last one zero.
                let last = (len % 8 != 0).then(|| 0xFF >> (8 - len % 8));
                for i in 0..count {
                    let byte = bytes.get(i).copied().unwrap_or(0);
                    let keep = if i + 1 == count { last.unwrap_or(0xFF) } else { 0xFF };
                    out.push(byte & keep);
                }
            }
        }
    }
}

/// What a bit stands for: 1 when it is set, -1 when it is not.
fn sign(byte: u8, bit: usize) -> f32 {
    if byte >> bit & 1 == 1 { 1.0 } else { -1.0 }
}

/// A vector inside a document.
#[derive(Clone, Copy, Debug)]
pub struct VectorRef<'a> {
    element: Element,
    /// The elements, without the bytes that say what they are.
    bytes: &'a [u8],
    len: usize,
}

impl<'a> ValueRef<'a> {
    /// The value as a vector, when it is one.
    pub fn as_vector(&self) -> Option<VectorRef<'a>> {
        match self.item() {
            Item::Ext(ext::VECTOR, Some(payload)) => VectorRef::parse(payload.as_data()?),
            _ => None,
        }
    }
}

/// The sum over the dimensions of `term(element, query)`, `K` sums at once.
///
/// The elements are taken 8 at a time into 8 sums that do not wait for one another, which
/// is what lets the compiler use the vector instructions of the machine: a loop with one
/// sum has to add in order, one dimension after another.
fn sums<const W: usize, const K: usize>(
    bytes: &[u8],
    query: &[f32],
    element: impl Fn([u8; W]) -> f32,
    term: impl Fn(f32, f32) -> [f32; K],
) -> [f32; K] {
    let (elements, _) = bytes.as_chunks::<W>();
    let mut lanes = [[0f32; 8]; K];
    let (blocks, rest) = elements.as_chunks::<8>();
    let (query_blocks, query_rest) = query.as_chunks::<8>();
    for (block, query) in blocks.iter().zip(query_blocks) {
        for lane in 0..8 {
            let terms = term(element(block[lane]), query[lane]);
            for k in 0..K {
                lanes[k][lane] += terms[k];
            }
        }
    }
    let mut total = lanes.map(|lane| lane.iter().sum::<f32>());
    for (bytes, query) in rest.iter().zip(query_rest) {
        let terms = term(element(*bytes), *query);
        for k in 0..K {
            total[k] += terms[k];
        }
    }
    total
}

impl<'a> VectorRef<'a> {
    /// The vector in the bytes of a payload, when they are one.
    pub(crate) fn parse(data: &'a [u8]) -> Option<Self> {
        let (&code, rest) = data.split_first()?;
        let element = Element::from_code(code)?;
        if element == Element::Bit {
            let (&unused, bytes) = rest.split_first()?;
            let len = (bytes.len() * 8).checked_sub(usize::from(unused))?;
            return (unused < 8).then_some(Self { element, bytes, len });
        }
        let size = element.bits() / 8;
        (rest.len() % size == 0).then_some(Self { element, bytes: rest, len: rest.len() / size })
    }

    pub fn element(&self) -> Element {
        self.element
    }

    /// The number of dimensions.
    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The bytes of the elements as they are stored, little-endian.
    pub fn as_bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The leading `dims` dimensions, which a Matryoshka model makes an embedding of their
    /// own, or `None` when the vector has fewer. Nothing is copied.
    pub fn prefix(&self, dims: usize) -> Option<VectorRef<'a>> {
        let bytes = self.bytes.get(..(dims * self.element.bits()).div_ceil(8))?;
        (dims <= self.len).then_some(Self { element: self.element, bytes, len: dims })
    }

    /// The sums over the dimensions of `term(element, query)`, or `None` when the query has
    /// another number of dimensions.
    fn sums<const K: usize>(
        &self,
        query: &[f32],
        term: impl Fn(f32, f32) -> [f32; K],
    ) -> Option<[f32; K]> {
        if query.len() != self.len {
            return None;
        }
        Some(match self.element {
            Element::F32 => sums(self.bytes, query, f32::from_le_bytes, term),
            Element::F64 => sums(self.bytes, query, |b| f64::from_le_bytes(b) as f32, term),
            Element::F16 => sums(self.bytes, query, |b| F16(u16::from_le_bytes(b)).to_f32(), term),
            Element::I8 => sums(self.bytes, query, |[b]| f32::from(b as i8) * I8_STEP, term),
            Element::Bit => {
                // A byte is 8 dimensions, which go into the 8 sums as above.
                let (blocks, rest) = query.as_chunks::<8>();
                let mut lanes = [[0f32; 8]; K];
                for (byte, query) in self.bytes.iter().zip(blocks) {
                    for lane in 0..8 {
                        let terms = term(sign(*byte, lane), query[lane]);
                        for k in 0..K {
                            lanes[k][lane] += terms[k];
                        }
                    }
                }
                let mut total = lanes.map(|lane| lane.iter().sum::<f32>());
                let last = self.bytes.get(blocks.len()).copied().unwrap_or(0);
                for (bit, query) in rest.iter().enumerate() {
                    let terms = term(sign(last, bit), *query);
                    for k in 0..K {
                        total[k] += terms[k];
                    }
                }
                total
            }
        })
    }

    /// The dot product with `query`, or `None` when it has another number of dimensions.
    /// For unit vectors this is their cosine.
    pub fn dot(&self, query: &[f32]) -> Option<f32> {
        self.sums(query, |element, query| [element * query]).map(|[dot]| dot)
    }

    /// The square of the distance to `query`.
    pub fn l2_squared(&self, query: &[f32]) -> Option<f32> {
        let square = |element: f32, query: f32| [(element - query) * (element - query)];
        self.sums(query, square).map(|[distance]| distance)
    }

    /// The cosine of the angle to `query`, whatever the lengths of the two: 0 when either
    /// has none.
    pub fn cosine(&self, query: &[f32]) -> Option<f32> {
        let terms = |element: f32, query: f32| [element * query, element * element, query * query];
        let [dot, own, other] = self.sums(query, terms)?;
        let lengths = (own * other).sqrt();
        Some(if lengths > 0.0 { dot / lengths } else { 0.0 })
    }

    /// The number of dimensions in which two vectors of bits differ, or `None` when they
    /// are not both of bits or of one number of dimensions.
    pub fn hamming(&self, other: &VectorRef<'_>) -> Option<u32> {
        if (self.element, other.element) != (Element::Bit, Element::Bit) || self.len != other.len {
            return None;
        }
        // The bits past the last dimension, which a prefix has set, do not count.
        let last = if self.len.is_multiple_of(8) { 0xFF } else { 0xFF >> (8 - self.len % 8) };
        let count = self.len.div_ceil(8);
        let pairs = self.bytes.iter().zip(other.bytes).take(count).enumerate();
        Some(
            pairs
                .map(|(i, (a, b))| {
                    ((a ^ b) & if i + 1 == count { last } else { 0xFF }).count_ones()
                })
                .sum(),
        )
    }

    /// The element at `index` as the value it stands for.
    pub fn get(&self, index: usize) -> Option<f32> {
        if index >= self.len {
            return None;
        }
        let at = |size: usize| self.bytes.get(index * size..(index + 1) * size);
        Some(match self.element {
            Element::F32 => f32::from_le_bytes(at(4)?.try_into().ok()?),
            Element::F64 => f64::from_le_bytes(at(8)?.try_into().ok()?) as f32,
            Element::F16 => F16(u16::from_le_bytes(at(2)?.try_into().ok()?)).to_f32(),
            Element::I8 => f32::from(*self.bytes.get(index)? as i8) * I8_STEP,
            Element::Bit => sign(*self.bytes.get(index / 8)?, index % 8),
        })
    }

    /// The values the elements stand for.
    pub fn to_f32(&self) -> Vec<f32> {
        (0..self.len).filter_map(|index| self.get(index)).collect()
    }

    /// The vector as one that owns its elements, of the same type.
    pub fn to_vector(&self) -> Vector {
        fn elements<const W: usize, T>(bytes: &[u8], element: impl Fn([u8; W]) -> T) -> Vec<T> {
            bytes.as_chunks::<W>().0.iter().map(|bytes| element(*bytes)).collect()
        }
        match self.element {
            Element::F32 => Vector::F32(elements(self.bytes, f32::from_le_bytes)),
            Element::F64 => Vector::F64(elements(self.bytes, f64::from_le_bytes)),
            Element::F16 => Vector::F16(elements(self.bytes, |b| F16(u16::from_le_bytes(b)))),
            Element::I8 => Vector::I8(self.bytes.iter().map(|b| *b as i8).collect()),
            Element::Bit => {
                // Through the writer's own rule, so that a prefix loses its extra bits.
                let all = Vector::Bits { bytes: self.bytes.to_vec(), len: self.len };
                let mut written = Vec::with_capacity(all.byte_len());
                all.write_bytes(&mut written);
                Vector::Bits { bytes: written.split_off(2), len: self.len }
            }
        }
    }
}
