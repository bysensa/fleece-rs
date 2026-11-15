//! What the fuzz targets share.

use fleece::{Value, encode};

/// The input cut in two: its first two bytes say how long the first part is. The seeds of
/// `examples/fuzz_seeds.rs` are joined the same way.
pub fn split(data: &[u8]) -> (&[u8], &[u8]) {
    let Some((head, rest)) = data.split_first_chunk::<2>() else { return (data, &[]) };
    rest.split_at(usize::from(u16::from_le_bytes(*head)).min(rest.len()))
}

/// The bytes a value is written as. Two values are compared by them: a NaN is not equal to
/// itself, and its bytes are.
pub fn bytes(value: &Value) -> Vec<u8> {
    encode(value).expect("a value that was read is written")
}

#[track_caller]
pub fn assert_same(left: &Value, right: &Value) {
    assert_eq!(bytes(left), bytes(right), "{left:?} is not {right:?}");
}
