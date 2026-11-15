//! A table of shared keys from any bytes, and a document read and written with it.
#![no_main]

use fleece::{Doc, Encoder, SharedKeys, Value};
use fleece_fuzz::{assert_same, split};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (state, document) = split(data);
    let Ok(keys) = SharedKeys::from_state(state) else { return };

    // The table stored again is the same table.
    let stored = keys.to_state().expect("a table that was read is stored");
    let again = SharedKeys::from_state(&stored).expect("a stored table is read");
    assert!(keys.keys().eq(again.keys()));

    // A key number the table lacks is refused by the check.
    let doc = Doc::trusted(document).with_shared_keys(&keys);
    if doc.validate().is_err() {
        return;
    }
    let value = Value::from_ref(doc.root()).expect("a checked document is decoded");

    let mut encoder = Encoder::new().with_fixed_shared_keys(&keys);
    encoder.write_value(&value).expect("a value that was read is written");
    let written = encoder.finish().expect("the document is finished");
    let doc = Doc::trusted(&written).with_shared_keys(&keys);
    doc.validate().expect("what was written passes the check");
    assert_same(&Value::from_ref(doc.root()).expect("and is decoded"), &value);
});
