//! Any bytes as a document: reading them never panics, and what a check accepted is a value
//! every part of the crate takes.
#![no_main]

use std::hint::black_box;

use fleece::key::encode_key;
use fleece::{Doc, Value, compact, decode, dump};
use fleece_fuzz::{assert_same, bytes};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Bytes nobody checked are read in place, and give a value or an error.
    let unchecked = Doc::trusted(data).root();
    if let Some(dict) = unchecked.as_dict() {
        let _ = (dict.get("name"), dict.len());
        dict.iter().for_each(|(_, value)| _ = black_box(value.item()));
    }
    if let Some(array) = unchecked.as_array() {
        array.iter().for_each(|value| _ = black_box(value.item()));
    }
    let _ = Value::from_ref(unchecked);

    let Ok(doc) = Doc::new(data) else { return };
    let root = doc.root();
    let value = Value::from_ref(root).expect("a checked document is decoded");

    // Written again and read again, it is the same value.
    let written = bytes(&value);
    assert_eq!(bytes(&decode(&written).expect("what was written is read")), written);

    // Compacted, it is the same value.
    let _ = doc.live_bytes();
    let compacted = compact(&doc).expect("a checked document is compacted");
    assert_same(&decode(&compacted).expect("a compacted document is read"), &value);

    // Through serde, it is the same value.
    let by_serde: Value = fleece::serde::from_slice(data).expect("serde reads a checked document");
    assert_same(&by_serde, &value);
    let from_serde = fleece::serde::to_vec(&value).expect("serde writes a value");
    assert_same(&decode(&from_serde).expect("what serde wrote is read"), &value);

    let _ = dump(&doc);
    let _ = encode_key(&value);
    let _ = fleece::json::to_json(&value);
    let _ = root.as_geo();

    if let Some(vector) = root.as_vector() {
        let query = vec![0.5; vector.len()];
        let _ = (vector.dot(&query), vector.cosine(&query), vector.l2_squared(&query));
        let _ = (vector.hamming(&vector), vector.get(0), vector.to_f32());
        let half = vector.prefix(vector.len() / 2);
        let _ = half.map(|half| half.to_vector());
        assert_same(&Value::Vector(vector.to_vector()), &value);
    }
});
