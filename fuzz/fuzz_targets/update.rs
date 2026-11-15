//! A document, a path and a value: the document changed at the path passes the check and
//! holds the value there; the part taken out at the path is what the path reads.
#![no_main]

use fleece::{Doc, KeyPath, Patch, Value, decode, extract, update};
use fleece_fuzz::{assert_same, split};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let (document, rest) = split(data);
    let (path, value) = split(rest);
    let Ok(doc) = Doc::new(document) else { return };
    let Some(path) = std::str::from_utf8(path).ok().and_then(|text| text.parse::<KeyPath>().ok())
    else {
        return;
    };
    let Ok(value) = decode(value) else { return };

    if let Ok(Some(part)) = extract(&doc, &path) {
        let there = path.eval(doc.root()).expect("what is taken out is at the path");
        let part = decode(&part).expect("the part taken out passes the check");
        assert_same(&part, &Value::from_ref(there).expect("a checked document is decoded"));
    }

    let mut patch = Patch::new();
    patch.set(&path, &value);
    if let Ok(updated) = update(&doc, &patch) {
        let updated = Doc::new(&updated).expect("an updated document passes the check");
        let there = path.eval(updated.root()).expect("the value set is at the path");
        assert_same(&Value::from_ref(there).expect("it is decoded"), &value);
    }

    let mut patch = Patch::new();
    patch.remove(&path);
    if let Ok(updated) = update(&doc, &patch) {
        Doc::new(&updated).expect("a document with a field removed passes the check");
    }
});
