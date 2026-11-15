//! The bytes written and read, against the documents of couchbase/fleece.

use fleece::{
    DecodeError, Doc, EncodeError, Encoder, Item, KeyPath, Number, Object, SharedKeys, Uuid, Value,
    decode, dump, encode,
};

fn hex(text: &str) -> Vec<u8> {
    text.split_whitespace().map(|byte| u8::from_str_radix(byte, 16).unwrap()).collect()
}

/// The output of `fleece --encode` in `Example.md` of couchbase/fleece.
const EXAMPLE: &str = "
    45 68 65 6c 6c 6f 46 77  6f 72 6c 64 21 00 44 74
    69 6d 65 00 1b d2 02 96  49 00 45 66 6c 6f 61 74
    28 00 f6 0b 76 c3 b6 45  89 3f 47 62 6f 6f 6c 65
    61 6e 49 6f 74 68 65 72  62 6f 6f 6c 44 6e 75 6c
    6c 00 43 6f 62 6a 44 77  68 61 74 00 44 74 68 61
    74 00 70 01 80 07 80 05  43 61 72 72 60 03 00 01
    00 02 00 03 70 08 80 07  80 06 80 20 38 00 80 2a
    80 28 80 39 80 37 80 1d  30 00 80 1c 80 15 80 26
    34 00 80 3a 80 38 80 11";

fn example() -> Result<Vec<u8>, EncodeError> {
    let mut encoder = Encoder::new();
    encoder.begin_dict()?;
    encoder.write_key("hello")?;
    encoder.write_str("world!")?;
    encoder.write_key("time")?;
    encoder.write_uint(1_234_567_890)?;
    encoder.write_key("float")?;
    encoder.write_f64(0.01234)?;
    encoder.write_key("boolean")?;
    encoder.write_bool(true)?;
    encoder.write_key("otherbool")?;
    encoder.write_bool(false)?;
    encoder.write_key("null")?;
    encoder.write_null()?;
    encoder.write_key("obj")?;
    encoder.begin_dict()?;
    encoder.write_key("what")?;
    encoder.write_str("that")?;
    encoder.end_dict()?;
    encoder.write_key("arr")?;
    encoder.begin_array()?;
    for item in 1..=3 {
        encoder.write_int(item)?;
    }
    encoder.end_array()?;
    encoder.end_dict()?;
    encoder.finish()
}

#[test]
fn writes_the_example_of_fleece_byte_for_byte() {
    assert_eq!(example().unwrap(), hex(EXAMPLE));
}

#[test]
fn reads_the_example_of_fleece() {
    let bytes = hex(EXAMPLE);
    let root = Doc::new(&bytes).unwrap().root();
    assert_eq!(root.position(), 0x64);
    let dict = root.as_dict().unwrap();
    assert_eq!(dict.len(), 8);
    assert_eq!(dict.get("hello").unwrap().as_str(), Some("world!"));
    assert_eq!(dict.get("time").unwrap().as_i64(), Some(1_234_567_890));
    assert_eq!(dict.get("float").unwrap().as_f64(), Some(0.01234));
    assert_eq!(dict.get("boolean").unwrap().as_bool(), Some(true));
    assert!(dict.get("null").unwrap().is_null());
    assert!(dict.get("missing").is_none());
    let array = dict.get("arr").unwrap().as_array().unwrap();
    assert_eq!(array.iter().map(|item| item.as_i64().unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
    assert!(array.get(3).is_none());
    let keys: Vec<&str> = dict.iter().map(|(key, _)| key).collect();
    assert_eq!(keys, ["arr", "boolean", "float", "hello", "null", "obj", "otherbool", "time"]);

    let path: KeyPath = "obj.what".parse().unwrap();
    assert_eq!(path.eval(root).unwrap().as_str(), Some("that"));
    let path: KeyPath = "$.arr[-1]".parse().unwrap();
    assert_eq!(path.eval(root).unwrap().as_i64(), Some(3));
    assert!("arr[3]".parse::<KeyPath>().unwrap().eval(root).is_none());
    assert!("hello.x".parse::<KeyPath>().unwrap().eval(root).is_none());

    let listing = dump(&Doc::trusted(&bytes));
    assert!(listing.contains("0064: Dict[8]\n0066:   &0058\n"), "{listing}");
    assert!(listing.contains("0014: 1234567890\n"), "{listing}");
    assert!(listing.ends_with("0086: &0064\n"), "{listing}");
}

#[test]
fn writes_the_small_example_of_fleece() {
    let value = Value::Object(Object::from([("foo".to_owned(), Value::from(123))]));
    assert_eq!(encode(&value).unwrap(), hex("43 66 6f 6f 70 01 80 03 00 7b 80 03"));
    // The wide form of the same dictionary, as `Fleece.md` gives it.
    let wide = hex("78 01 43 66 6f 6f 00 7b 00 00 80 05");
    assert_eq!(decode(&wide).unwrap(), value);
}

#[test]
fn extended_values_follow_the_byte_0x31() {
    assert_eq!(encode(&Value::None).unwrap(), [0x31, 0x00]);
    assert_eq!(encode(&Value::Null).unwrap(), [0x30, 0x00]);
    let small = Value::Number(Number::U8(7));
    assert_eq!(encode(&small).unwrap(), [0x31, 0x02, 0x00, 0x07, 0x80, 0x02]);
    let uuid = encode(&Value::Uuid(Uuid([0xAB; 16]))).unwrap();
    assert_eq!(uuid[..4], [0x31, 0x0D, 0x5F, 0x10]);
    assert_eq!(uuid.len(), 22);
    // A float keeps its width, and a double that fits 4 bytes takes them.
    assert_eq!(encode(&Value::Number(Number::F32(1.5))).unwrap()[..2], [0x20, 0x00]);
    assert_eq!(encode(&Value::from(1.5)).unwrap()[..2], [0x24, 0x00]);
    assert_eq!(decode(&encode(&Value::from(1.5)).unwrap()).unwrap(), Value::from(1.5));
    assert_eq!(encode(&Value::from(0.1)).unwrap()[..2], [0x28, 0x00]);
}

#[test]
fn reads_a_dictionary_that_inherits() {
    // {"a": 1, "b": 2}, then a dictionary over it that deletes "b" and adds "c".
    let parent = "70 02 41 61 00 01 41 62 00 02";
    let child = "70 03 08 00 80 07 41 62 3c 00 41 63 00 03";
    let bytes = hex(&format!("{parent} {child} 80 07"));
    let dict = Doc::new(&bytes).unwrap().root().as_dict().unwrap();
    assert_eq!(dict.get("a").unwrap().as_i64(), Some(1));
    assert!(dict.get("b").is_none());
    assert_eq!(dict.get("c").unwrap().as_i64(), Some(3));
    assert_eq!(dict.len(), 2);
    let expected = Object::from([("a".to_owned(), 1.into()), ("c".to_owned(), 3.into())]);
    assert_eq!(decode(&bytes).unwrap(), Value::Object(expected));
}

#[test]
fn large_collections_go_wide() {
    // More bytes of strings than a narrow pointer reaches over.
    let strings: Vec<Value> = (0..3000).map(|i| format!("a string number {i:06}").into()).collect();
    let value = Value::Array(strings);
    let bytes = encode(&value).unwrap();
    let doc = Doc::new(&bytes).unwrap();
    let Item::Array(array) = doc.root().item() else { panic!() };
    assert_eq!(array.len(), 3000);
    assert_eq!(bytes[doc.root().position()] & 0x08, 0x08, "the array is wide");
    assert_eq!(array.get(2999).unwrap().as_str(), Some("a string number 002999"));
    assert_eq!(decode(&bytes).unwrap(), value);

    // The root lies farther back than the narrow pointer at the end reaches.
    let value = Value::Array((0..40_000).map(|i| Value::from(i % 100)).collect());
    let bytes = encode(&value).unwrap();
    assert_eq!(bytes[bytes.len() - 2..], [0x80, 0x02]);
    assert_eq!(decode(&bytes).unwrap(), value);
}

#[test]
fn shared_keys_shrink_documents_and_are_needed_to_read_them() {
    let value = Value::Object(Object::from([
        ("first_name".to_owned(), Value::from("Tintin")),
        ("last-name".to_owned(), Value::from("")),
        ("not eligible".to_owned(), Value::from(1)),
    ]));
    let mut shared = SharedKeys::new();
    let mut encoder = Encoder::new().with_shared_keys(&mut shared);
    encoder.write_value(&value).unwrap();
    let bytes = encoder.finish().unwrap();
    assert_eq!(shared.keys().collect::<Vec<_>>(), ["first_name", "last-name"]);
    assert!(bytes.len() < encode(&value).unwrap().len());

    let doc = Doc::trusted(&bytes).with_shared_keys(&shared);
    doc.validate().unwrap();
    assert_eq!(Value::from_ref(doc.root()).unwrap(), value);
    let dict = doc.root().as_dict().unwrap();
    assert_eq!(dict.get("first_name").unwrap().as_str(), Some("Tintin"));
    assert_eq!(dict.get("not eligible").unwrap().as_i64(), Some(1));
    assert!(dict.get("other").is_none());
    assert!(matches!(Doc::new(&bytes), Err(DecodeError::MissingSharedKey { key: 0, .. })));

    let restored = SharedKeys::from_state(&shared.to_state().unwrap()).unwrap();
    assert_eq!(restored, shared);
}

#[test]
fn the_encoder_refuses_calls_out_of_order() {
    let mut encoder = Encoder::new();
    assert_eq!(encoder.write_key("a"), Err(EncodeError::UnexpectedKey));
    encoder.begin_dict().unwrap();
    assert_eq!(encoder.write_int(1), Err(EncodeError::ExpectedKey));
    assert_eq!(encoder.end_array(), Err(EncodeError::Unbalanced));
    encoder.write_key("a").unwrap();
    assert_eq!(encoder.end_dict(), Err(EncodeError::Unbalanced));
    assert_eq!(encoder.write_key("b"), Err(EncodeError::UnexpectedKey));
    encoder.write_int(1).unwrap();
    encoder.write_key("a").unwrap();
    encoder.write_int(2).unwrap();
    assert_eq!(encoder.end_dict(), Err(EncodeError::DuplicateKey("a".to_owned())));

    let mut encoder = Encoder::new();
    assert_eq!(encoder.begin_ext(0), Err(EncodeError::BadExtKind(0)));
    assert_eq!(encoder.begin_ext(200), Err(EncodeError::BadExtKind(200)));
    encoder.write_null().unwrap();
    assert_eq!(encoder.write_null(), Err(EncodeError::MultipleRoots));
    assert_eq!(Encoder::new().finish(), Err(EncodeError::Incomplete));

    let mut encoder = Encoder::new();
    for _ in 0..fleece::MAX_DEPTH {
        encoder.begin_array().unwrap();
    }
    assert_eq!(encoder.begin_array(), Err(EncodeError::TooDeep));
}

#[test]
fn documents_that_are_not_well_formed_are_refused() {
    let refused = |text: &str| Doc::new(&hex(text)).unwrap_err();
    assert_eq!(Doc::new(&[]).unwrap_err(), DecodeError::Truncated);
    assert_eq!(refused("00"), DecodeError::Truncated);
    // A pointer that leaves the data, and one that leads nowhere.
    assert_eq!(refused("80 05"), DecodeError::BadPointer { pos: 0 });
    assert_eq!(refused("80 00"), DecodeError::BadPointer { pos: 0 });
    // A string cut short, and one that is not UTF-8.
    assert_eq!(refused("45 68 80 01"), DecodeError::Malformed { pos: 0 });
    assert_eq!(refused("42 ff fe 00 80 02"), DecodeError::InvalidUtf8 { pos: 0 });
    // Keys out of order, and a key that is not a string.
    assert_eq!(
        refused("70 02 41 62 00 01 41 61 00 02 80 05"),
        DecodeError::UnsortedKeys { pos: 0 }
    );
    assert_eq!(refused("70 01 30 00 00 01 80 03"), DecodeError::BadKey { pos: 0 });
    // An unknown kind, a kind without its payload, and `undefined` as a value.
    assert_eq!(refused("31 c8 00 01 80 02"), DecodeError::UnknownExt { pos: 0, kind: 200 });
    assert_eq!(refused("31 0d"), DecodeError::BadPayload { pos: 0, kind: 13 });
    assert_eq!(
        decode(&hex("31 0d 00 01 80 02")),
        Err(DecodeError::BadPayload { pos: 0, kind: 13 })
    );
    assert_eq!(decode(&hex("3c 00")), Err(DecodeError::Undefined { pos: 0 }));
    // A pointer marked as leaving the document, without a base to land in.
    assert_eq!(refused("00 01 60 01 c0 02 80 02"), DecodeError::BadPointer { pos: 4 });
}

/// Everything that walks a document, on the deepest documents there are, in a thread of
/// half the stack a thread gets by default (1 MiB of 2) and in a build without
/// optimization, where every call takes the most stack it ever does.
#[test]
fn the_deepest_documents_are_walked_without_running_out_of_stack() {
    use std::ops::Bound;

    let deep = |wrap: &dyn Fn(Value) -> Value, levels: usize| {
        (0..levels).fold(Value::from(1), |inner, _| wrap(inner))
    };
    let range = |inner: Value| {
        Value::Range(Box::new(fleece::Range {
            start: Bound::Included(inner),
            end: Bound::Unbounded,
        }))
    };
    let record = |inner: Value| {
        let key = fleece::RecordIdKey::Array(vec![inner]);
        Value::RecordId(fleece::RecordId { table: "t".into(), key })
    };
    let object = |inner: Value| Value::Object(Object::from([("k".to_owned(), inner)]));
    let mut geometry = fleece::Geometry::Point(fleece::Point { x: 1.0, y: 2.0 });
    // A collection of geometries is two collections, and the point in the middle one.
    for _ in 0..fleece::MAX_DEPTH / 2 - 1 {
        geometry = fleece::Geometry::Collection(vec![geometry]);
    }
    let values = [
        deep(&|inner| Value::Array(vec![inner]), fleece::MAX_DEPTH),
        deep(&object, fleece::MAX_DEPTH),
        deep(&|inner| Value::Set(vec![inner]), fleece::MAX_DEPTH),
        deep(&range, fleece::MAX_DEPTH),
        // A record with an array for its key is two collections.
        deep(&record, fleece::MAX_DEPTH / 2),
        Value::Geometry(geometry),
    ];
    // Bytes that are not a document one may write: arrays nested far past the limit, as
    // only bytes that are not trusted hold them. Reading them ends, in an error or short.
    let mut too_deep = vec![0x00, 0x01];
    for level in 0..5000 {
        // An array of one item: the integer the first time, then the array before it.
        too_deep.extend_from_slice(&[0x60, 0x01, 0x80, if level == 0 { 0x02 } else { 0x03 }]);
    }
    too_deep.extend_from_slice(&[0x80, 0x02]);
    let walk = move || {
        assert_eq!(Doc::new(&too_deep).unwrap_err(), DecodeError::TooDeep);
        let unchecked = Doc::trusted(&too_deep);
        assert_eq!(Value::from_ref(unchecked.root()), Err(DecodeError::TooDeep));
        assert!(!dump(&unchecked).is_empty());
        assert!(unchecked.live_bytes() <= too_deep.len());
        assert!(fleece::compact(&unchecked).is_err());
        assert!(fleece::update(&unchecked, &fleece::Patch::new()).is_err());
        for value in &values {
            let bytes = encode(value).unwrap();
            let doc = Doc::new(&bytes).unwrap();
            assert_eq!(&Value::from_ref(doc.root()).unwrap(), value);
            assert!(!dump(&doc).is_empty());
            assert_eq!(doc.live_bytes(), bytes.len());
            assert_eq!(fleece::compact(&doc).unwrap(), bytes);
            assert_eq!(fleece::update(&doc, &fleece::Patch::new()).unwrap(), bytes);
            assert!(fleece::Kind::Any.conforms(doc.root()));
            assert!(!fleece::key::encode_key(value).is_empty());
            #[cfg(feature = "json")]
            assert_eq!(&fleece::json::from_json(&fleece::json::to_json(value)).unwrap(), value);
            // One level more is refused, not attempted.
            let deeper = Value::Array(vec![value.clone()]);
            if matches!(value, Value::Array(_) | Value::Object(_) | Value::Set(_)) {
                assert_eq!(encode(&deeper), Err(EncodeError::TooDeep));
            }
        }
    };
    let thread = std::thread::Builder::new().stack_size(1 << 20).spawn(walk).unwrap();
    thread.join().unwrap();
}

/// An array of one string, inside `levels` arrays that each hold the one before it `width`
/// times, by pointers to the same bytes.
fn held_many_times(levels: usize, width: u8) -> Vec<u8> {
    let mut encoder = Encoder::new();
    encoder.begin_array().unwrap();
    encoder.write_str("some string").unwrap();
    encoder.end_array().unwrap();
    let mut bytes = encoder.finish().unwrap();
    bytes.truncate(bytes.len() - 2);
    let mut last = bytes.len() - 4;
    for _ in 0..levels {
        let at = bytes.len();
        bytes.extend_from_slice(&[0x60, width]);
        for slot in 0..usize::from(width) {
            let units = (at + 2 + 2 * slot - last) / 2;
            bytes.extend_from_slice(&(0x8000 | units as u16).to_be_bytes());
        }
        last = at;
    }
    let units = (bytes.len() - last) / 2;
    bytes.extend_from_slice(&(0x8000 | units as u16).to_be_bytes());
    bytes
}

#[test]
fn a_small_document_cannot_stand_for_an_endless_one() {
    // 16^10 values in 400 bytes.
    assert_eq!(Doc::new(&held_many_times(10, 16)).unwrap_err(), DecodeError::TooManyValues);
    // A million values in 200 bytes, and 2^20 of them in 150: few enough for a limit that
    // does not ask how long the document is, and a gigabyte once they are decoded.
    assert_eq!(Doc::new(&held_many_times(5, 16)).unwrap_err(), DecodeError::TooManyValues);
    assert_eq!(Doc::new(&held_many_times(20, 2)).unwrap_err(), DecodeError::TooManyValues);
    // An array held twice is within what the bytes of the document have room for.
    let twice = held_many_times(1, 2);
    let inner = Value::Array(vec!["some string".into()]);
    assert_eq!(decode(&twice).unwrap(), Value::Array(vec![inner.clone(), inner]));
}

#[test]
fn bytes_nobody_checked_end_every_read() {
    // A dictionary whose pointer to the one it inherits from leads to itself.
    let bytes = hex("70 01 08 00 80 02 80 03");
    assert_eq!(Doc::new(&bytes).unwrap_err(), DecodeError::TooDeep);
    let root = Doc::trusted(&bytes).root();
    let dict = root.as_dict().unwrap();
    assert!(dict.get("name").is_none());
    assert_eq!((dict.len(), dict.iter().count()), (0, 0));
    assert_eq!(Value::from_ref(root), Ok(Value::Object(Object::new())));

    // Arrays that hold one another many times make no more values than the bytes have
    // room for, also when no check stood in the way.
    let endless = held_many_times(40, 2);
    let decoded = Value::from_ref(Doc::trusted(&endless).root());
    assert_eq!(decoded, Err(DecodeError::TooManyValues));
}

#[test]
fn what_a_dictionary_hides_is_not_counted_against_the_document() {
    // A dictionary as full of values as its bytes have room for: every key and every value
    // lies in its slot. Each change is appended as a dictionary that inherits, the old
    // dictionaries staying in the bytes below it.
    let keys = || ('0'..='z').map(String::from);
    let inner = |first: i64| {
        let fields = keys().enumerate().map(|(i, key)| (key, Value::from(i as i64 + first)));
        Value::Object(fields.collect())
    };
    // Enough fields around it that a change of one is appended as a dictionary that
    // inherits the rest, so the dictionary before the change stays in reach of the check.
    let outer = |first: i64| {
        let around = ('a'..='h').map(|key| (key.to_string(), Value::from(1)));
        Value::Object(around.chain([("i".to_owned(), inner(first))]).collect())
    };
    let mut bytes = encode(&outer(0)).unwrap();
    for change in 1..40 {
        // One value changes on odd turns, every value on even ones.
        let mut new = outer(change / 2 * 100);
        if let (1, Value::Object(fields)) = (change % 2, &mut new) {
            let Some(Value::Object(inner)) = fields.get_mut("i") else { unreachable!() };
            inner.insert("0".to_owned(), Value::from(change));
        }
        let delta = fleece::delta::append_delta(&bytes, &new).unwrap();
        bytes.extend_from_slice(&delta);
        assert_eq!(decode(&bytes).unwrap(), new, "after {change} changes");
    }
}

#[test]
fn a_document_that_passes_the_check_is_one_decode_takes() {
    // An i8 of 1784.
    let wide = hex("31 01 06 f8 80 02");
    assert_eq!(Doc::new(&wide).unwrap_err(), DecodeError::BadPayload { pos: 0, kind: 1 });
    // A time of day past the end of the day, a UUID of three bytes, a table that is a number.
    for (bytes, kind) in [
        ("31 0f 1f ff ff ff ff ff ff ff ff 00 80 06", 15),
        ("31 0d 53 01 02 03 80 03", 13),
        ("31 12 00 07 80 02", 18),
    ] {
        let error = Doc::new(&hex(bytes)).unwrap_err();
        assert_eq!(error, DecodeError::BadPayload { pos: 0, kind }, "{bytes}");
    }
    // Undefined, which only deletes an entry of a dictionary inherited from.
    assert_eq!(Doc::new(&hex("3c 00")).unwrap_err(), DecodeError::Undefined { pos: 0 });
    let in_array = hex("60 01 3c 00 80 02");
    assert_eq!(Doc::new(&in_array).unwrap_err(), DecodeError::Undefined { pos: 2 });
    // Unchecked, such bytes still read, as what they are.
    assert!(matches!(Doc::trusted(&hex("3c 00")).root().item(), Item::Undefined));
}
