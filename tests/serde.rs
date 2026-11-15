//! The types of an application through `serde`.
#![cfg(feature = "serde")]

use std::collections::BTreeMap;

use fleece::serde::{Error, from_ref, from_slice, to_encoder, to_vec};
use fleece::{
    Datetime, Decimal, Doc, Encoder, Number, Object, RecordId, RecordIdKey, SharedKeys, Uuid,
    Value, decode, encode,
};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
enum Coat {
    White,
    Spotted(u8),
    Dyed { colour: String, washable: bool },
    Mixed(u8, u8),
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
struct Dog<'a> {
    name: &'a str,
    #[serde(borrow, with = "serde_bytes_like")]
    chip: &'a [u8],
    age: u8,
    weight: f32,
    height: f64,
    owner: Option<String>,
    vet: Option<String>,
    coat: Coat,
    coats: Vec<Coat>,
    scores: BTreeMap<String, i64>,
    by_year: BTreeMap<u16, bool>,
    pair: (i32, char),
    unit: (),
    id: Uuid,
    born: Datetime,
    price: Decimal,
    record: RecordId,
    anything: Value,
    big: u64,
}

/// Bytes as bytes: without it `serde` takes `&[u8]` for a sequence of numbers.
mod serde_bytes_like {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(bytes)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<&'de [u8], D::Error> {
        <&'de [u8]>::deserialize(deserializer)
    }
}

fn dog() -> Dog<'static> {
    Dog {
        name: "Milou",
        chip: &[0xDE, 0xAD],
        age: 9,
        weight: 7.5,
        height: 0.1,
        owner: Some("Tintin".into()),
        vet: None,
        coat: Coat::White,
        coats: vec![
            Coat::Spotted(3),
            Coat::Dyed { colour: "blue".into(), washable: true },
            Coat::Mixed(1, 2),
        ],
        scores: BTreeMap::from([("fetch".into(), 10), ("sit".into(), -2)]),
        by_year: BTreeMap::from([(2025, true)]),
        pair: (-7, 'é'),
        unit: (),
        id: Uuid([7; 16]),
        born: Datetime { secs: 1_500_000_000, nanos: 5 },
        price: "12.50".parse().unwrap(),
        record: RecordId { table: "dog".into(), key: RecordIdKey::String("milou".into()) },
        anything: Value::Set(vec![1.into(), Value::None]),
        big: u64::MAX,
    }
}

#[test]
fn a_struct_reads_back_as_written() {
    let dog = dog();
    let bytes = to_vec(&dog).unwrap();
    assert_eq!(from_slice::<Dog>(&bytes).unwrap(), dog);
}

#[test]
fn what_is_written_is_the_value_one_would_write_by_hand() {
    let bytes = to_vec(&dog()).unwrap();
    let Value::Object(fields) = decode(&bytes).unwrap() else { panic!() };
    // Integers are the integer of Fleece whatever their width, and floats keep theirs.
    assert_eq!(fields["age"], Value::from(9));
    assert_eq!(fields["weight"], Value::Number(Number::F32(7.5)));
    assert_eq!(fields["height"], Value::from(0.1));
    assert_eq!(fields["big"], Value::Number(Number::U64(u64::MAX)));
    assert_eq!(fields["chip"], Value::Bytes(vec![0xDE, 0xAD]));
    assert_eq!(fields["vet"], Value::Null);
    assert_eq!(fields["unit"], Value::Null);
    assert_eq!(fields["pair"], Value::Array(vec![(-7).into(), "é".into()]));
    // A variant without data is its name, one with data a dictionary of the name.
    assert_eq!(fields["coat"], Value::from("White"));
    let spotted = Object::from([("Spotted".to_owned(), 3.into())]);
    let Value::Array(coats) = &fields["coats"] else { panic!() };
    assert_eq!(coats[0], Value::Object(spotted));
    assert_eq!(fields["by_year"], Value::Object(Object::from([("2025".into(), true.into())])));
    // The types of this crate are the extended values they are.
    assert_eq!(fields["id"], Value::Uuid(Uuid([7; 16])));
    assert_eq!(fields["born"], Value::Datetime(Datetime { secs: 1_500_000_000, nanos: 5 }));
    assert_eq!(fields["price"], Value::Number(Number::Decimal("12.50".parse().unwrap())));
    assert_eq!(fields["anything"], Value::Set(vec![1.into(), Value::None]));
}

#[test]
fn strings_and_bytes_are_lent_out_of_the_document() {
    let bytes = to_vec(&dog()).unwrap();
    let dog: Dog = from_slice(&bytes).unwrap();
    let inside = |part: &[u8]| bytes.as_ptr_range().contains(&part.as_ptr());
    assert!(inside(dog.name.as_bytes()));
    assert!(inside(dog.chip));
}

#[test]
fn a_value_written_by_hand_is_read_as_a_struct() {
    #[derive(Deserialize, PartialEq, Debug)]
    struct Small {
        name: String,
        age: u8,
        tall: f32,
        tags: Vec<String>,
        nick: Option<String>,
    }
    let value = Value::Object(Object::from([
        ("name".to_owned(), "Milou".into()),
        // A sized number of SurrealDB is read as the integer it is, an f16 as a float,
        // a set as a sequence, `NONE` as nothing.
        ("age".to_owned(), Value::Number(Number::U8(9))),
        ("tall".to_owned(), Value::Number(Number::F16(fleece::F16(0x3E00)))),
        ("tags".to_owned(), Value::Set(vec!["dog".into()])),
        ("nick".to_owned(), Value::None),
        ("more".to_owned(), Value::Uuid(Uuid([1; 16]))),
    ]));
    let bytes = encode(&value).unwrap();
    let small: Small = from_slice(&bytes).unwrap();
    let expected =
        Small { name: "Milou".into(), age: 9, tall: 1.5, tags: vec!["dog".into()], nick: None };
    assert_eq!(small, expected);

    // A part of a document is read on its own.
    let name = Doc::new(&bytes).unwrap().root().as_dict().unwrap().get("name").unwrap();
    assert_eq!(from_ref::<&str>(name), Ok("Milou"));
}

#[test]
fn what_does_not_fit_is_refused_with_a_reason() {
    #[derive(Deserialize, Debug)]
    #[allow(dead_code)]
    struct Small {
        age: u8,
    }
    let wrong = |value: Value| {
        let bytes = encode(&Value::Object(Object::from([("age".to_owned(), value)]))).unwrap();
        from_slice::<Small>(&bytes).unwrap_err()
    };
    assert!(matches!(wrong(300.into()), Error::Message(_)));
    assert!(matches!(wrong("nine".into()), Error::Message(_)));
    // A typed value is not taken for something else.
    let Error::Message(reason) = wrong(Value::Datetime(Datetime { secs: 0, nanos: 0 })) else {
        panic!()
    };
    assert!(reason.contains("datetime"), "{reason}");
    assert!(matches!(from_slice::<Small>(&[0x80]), Err(Error::Decode(_))));
    assert_eq!(from_slice::<Uuid>(&encode(&1.into()).unwrap()).ok(), None);
    // The key of a map has to be a string or what is written as one.
    assert!(to_vec(&BTreeMap::from([(true, 1)])).is_err());
    assert!(to_vec(&u128::MAX).is_err());
}

#[test]
fn structs_are_written_with_shared_keys() {
    let dog = dog();
    let mut shared = SharedKeys::new();
    let mut encoder = Encoder::new().with_shared_keys(&mut shared);
    to_encoder(&dog, &mut encoder).unwrap();
    let bytes = encoder.finish().unwrap();
    assert!(bytes.len() < to_vec(&dog).unwrap().len());
    assert!(shared.encode("name").is_some());
    let doc = Doc::trusted(&bytes).with_shared_keys(&shared);
    assert_eq!(from_ref::<Dog>(doc.root()).unwrap(), dog);
}
