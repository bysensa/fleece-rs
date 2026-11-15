//! Two documents: the difference between them, applied to the first, is the second, as a
//! value and as bytes appended to the first.
#![no_main]

use fleece::delta::{append_delta, apply, diff};
use fleece::{Doc, Value, compact, decode};
use fleece_fuzz::{assert_same, split};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (first, second) = split(data);
    let (Ok(old), Ok(new)) = (decode(first), decode(second)) else { return };

    // A value that is no delta is refused, or gives some value.
    let _ = apply(&old, &new);

    match diff(&old, &new) {
        Some(delta) => assert_same(&apply(&old, &delta).expect("a delta of diff is applied"), &new),
        None => assert_same(&old, &new),
    }

    let appended = append_delta(first, &new).expect("a delta is appended to a checked document");
    let whole = [first, &appended[..]].concat();
    let doc = Doc::new(&whole).expect("a document with an appended delta passes the check");
    assert_same(&Value::from_ref(doc.root()).expect("it is decoded"), &new);
    let compacted = compact(&doc).expect("it is compacted");
    assert_same(&decode(&compacted).expect("the compacted document is read"), &new);
});
