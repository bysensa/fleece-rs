//! Any text as JSON: the value it gives is written and read back the same, and so is the
//! JSON of that value.
#![no_main]

use fleece::json::{from_json, json_to_fleece, to_json};
use fleece::{decode, encode};
use fleece_fuzz::assert_same;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = json_to_fleece(text);
    let Ok(json) = serde_json::from_str::<serde_json::Value>(text) else { return };
    let Ok(value) = from_json(&json) else { return };
    let Ok(written) = encode(&value) else { return };
    assert_same(&decode(&written).expect("what was written is read"), &value);
    assert_same(&from_json(&to_json(&value)).expect("the JSON of a value is read"), &value);
});
