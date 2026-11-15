//! What a store of documents asks of the format: a change without a decode, a part of a
//! document, the kind of a value, shared keys that can be taken back.

use std::collections::BTreeMap;

use fleece::{
    DeltaError, Doc, Encoder, Geometry, GeometryKind, Kind, KindLiteral, Number, Patch, Point,
    RecordId, RecordIdKey, SharedKeys, Uuid, Value, decode, encode, extract, update,
};

fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Object(fields.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())
}

fn user() -> Value {
    object([
        ("name", "Milou".into()),
        ("age", Value::Number(Number::U8(9))),
        ("id", Value::Uuid(Uuid([7; 16]))),
        ("tags", vec!["dog".into(), "white".into()].into()),
        ("address", object([("city", "Brussels".into()), ("zip", "1000".into())])),
    ])
}

/// The document of `value` with the changes: `Some` sets the value at a path, `None`
/// removes what is there.
fn changed(value: &Value, changes: &[(&str, Option<Value>)]) -> Result<Value, DeltaError> {
    let bytes = encode(value).unwrap();
    let paths: Vec<_> = changes.iter().map(|(path, _)| path.parse().unwrap()).collect();
    let mut patch = Patch::new();
    for (path, (_, change)) in paths.iter().zip(changes) {
        match change {
            Some(value) => patch.set(path, value),
            None => patch.remove(path),
        };
    }
    let new = update(&Doc::new(&bytes).unwrap(), &patch)?;
    Ok(decode(&new).unwrap())
}

#[test]
fn a_document_is_changed_by_paths() {
    let user = user();
    let new = changed(
        &user,
        &[
            ("age", Some(Value::Number(Number::U8(10)))),
            ("address.city", Some("Moulinsart".into())),
            ("address.zip", None),
            ("tags[0]", None),
            ("tags[2]", Some("small".into())),
            ("owner.name", Some("Tintin".into())),
            ("missing", None),
        ],
    )
    .unwrap();
    let expected = object([
        ("name", "Milou".into()),
        ("age", Value::Number(Number::U8(10))),
        ("id", Value::Uuid(Uuid([7; 16]))),
        ("tags", vec!["white".into(), "small".into()].into()),
        ("address", object([("city", "Moulinsart".into())])),
        ("owner", object([("name", "Tintin".into())])),
    ]);
    assert_eq!(new, expected);

    // What an update writes is what writing the new value gives: nothing is left over.
    let bytes = encode(&user).unwrap();
    let mut patch = Patch::new();
    let (path, ten) = ("age".parse().unwrap(), Value::Number(Number::U8(10)));
    patch.set(&path, &ten);
    let updated = update(&Doc::new(&bytes).unwrap(), &patch).unwrap();
    let Value::Object(mut fields) = user.clone() else { panic!() };
    fields.insert("age".to_owned(), ten.clone());
    assert_eq!(updated.len(), encode(&Value::Object(fields)).unwrap().len());

    assert_eq!(changed(&user, &[]), Ok(user.clone()));
    assert_eq!(changed(&user, &[("tags[-1]", Some("black".into()))]).unwrap(), {
        let Value::Object(mut fields) = user.clone() else { panic!() };
        fields.insert("tags".to_owned(), vec!["dog".into(), "black".into()].into());
        Value::Object(fields)
    });
    // A whole value is replaced at the root, and the later of two changes stays.
    assert_eq!(changed(&user, &[("", Some(1.into()))]), Ok(1.into()));
    assert_eq!(changed(&user, &[("age", None), ("age", Some(1.into()))]).unwrap(), {
        let Value::Object(mut fields) = user.clone() else { panic!() };
        fields.insert("age".to_owned(), 1.into());
        Value::Object(fields)
    });
}

#[test]
fn a_path_that_does_not_fit_is_refused() {
    let user = user();
    for path in ["name.first", "tags.first", "tags[3]", "tags[-3]", "address[0]", "age[0]"] {
        let refused = changed(&user, &[(path, Some(1.into()))]);
        assert_eq!(refused, Err(DeltaError::Mismatch), "{path}");
    }
    assert_eq!(changed(&user, &[("", None)]), Err(DeltaError::Mismatch));
}

#[test]
fn a_part_of_a_document_is_a_document() {
    let bytes = encode(&user()).unwrap();
    let doc = Doc::new(&bytes).unwrap();
    let part = extract(&doc, &"address".parse().unwrap()).unwrap().unwrap();
    let address = object([("city", "Brussels".into()), ("zip", "1000".into())]);
    assert_eq!(decode(&part).unwrap(), address);
    assert_eq!(part, encode(&address).unwrap());
    let tag = extract(&doc, &"tags[1]".parse().unwrap()).unwrap().unwrap();
    assert_eq!(decode(&tag).unwrap(), Value::from("white"));
    assert_eq!(extract(&doc, &"address.country".parse().unwrap()), Ok(None));
}

#[test]
fn changes_and_parts_keep_shared_keys() {
    let mut shared = SharedKeys::new();
    let mut encoder = Encoder::new().with_shared_keys(&mut shared);
    encoder.write_value(&user()).unwrap();
    let bytes = encoder.finish().unwrap();
    let doc = Doc::trusted(&bytes).with_shared_keys(&shared);

    let (path, value) = ("address.city".parse().unwrap(), Value::from("Moulinsart"));
    let mut patch = Patch::new();
    patch.set(&path, &value);
    let updated = update(&doc, &patch).unwrap();
    let new = Doc::trusted(&updated).with_shared_keys(&shared);
    new.validate().unwrap();
    let city = "address.city".parse::<fleece::KeyPath>().unwrap().eval(new.root()).unwrap();
    assert_eq!(city.as_str(), Some("Moulinsart"));
    // The keys are the numbers they were: no key is spelled out in the new bytes.
    assert!(!updated.windows(4).any(|window| window == b"name"));

    let part = extract(&doc, &"address".parse().unwrap()).unwrap().unwrap();
    let part = Doc::trusted(&part).with_shared_keys(&shared);
    assert_eq!(part.root().as_dict().unwrap().get("zip").unwrap().as_str(), Some("1000"));
}

#[test]
fn keys_added_by_a_write_that_failed_are_taken_back() {
    let mut shared = SharedKeys::new();
    shared.encode_and_add("name");
    let before = shared.len();
    let mut encoder = Encoder::new().with_shared_keys(&mut shared);
    encoder.write_value(&user()).unwrap();
    drop(encoder.finish());
    assert_eq!(shared.len(), 7);

    shared.truncate(before);
    assert_eq!(shared.keys().collect::<Vec<_>>(), ["name"]);
    assert_eq!(shared.encode("age"), None);
    // The numbers are free again for whoever writes next.
    assert_eq!(shared.encode_and_add("other"), Some(1));
    shared.truncate(100);
    assert_eq!(shared.len(), 2);
}

fn conforms(kind: &Kind, value: &Value) -> bool {
    let bytes = encode(value).unwrap();
    kind.conforms(Doc::new(&bytes).unwrap().root())
}

#[test]
fn a_value_is_of_its_kind_and_no_other() {
    let record = Value::RecordId(RecordId { table: "user".into(), key: RecordIdKey::Number(1) });
    let point = Value::Geometry(Geometry::Point(Point { x: 1.0, y: 2.0 }));
    let cases = [
        (Kind::None, Value::None),
        (Kind::Null, Value::Null),
        (Kind::Bool, true.into()),
        (Kind::Int, 1.into()),
        (Kind::Float, 1.5.into()),
        (Kind::F32, Value::Number(Number::F32(1.5))),
        (Kind::U8, Value::Number(Number::U8(1))),
        (Kind::I32, Value::Number(Number::I32(1))),
        (Kind::U64, Value::Number(Number::U64(u64::MAX))),
        (Kind::Decimal, Value::Number(Number::Decimal("1.5".parse().unwrap()))),
        (Kind::BigDecimal, Value::Number(Number::BigDecimal("1.5".parse().unwrap()))),
        (Kind::String, "text".into()),
        (Kind::Bytes, Value::Bytes(vec![1])),
        (Kind::Uuid, Value::Uuid(Uuid([1; 16]))),
        (Kind::Object, user()),
        (Kind::Regex, Value::Regex("a+".into())),
        (Kind::Table(vec![]), Value::Table("user".into())),
        (Kind::Record(vec!["user".into()]), record.clone()),
        (Kind::Geometry(vec![GeometryKind::Point]), point.clone()),
        (Kind::Array(Box::new(Kind::Int), None), vec![1.into(), 2.into()].into()),
        (Kind::Set(Box::new(Kind::String), None), Value::Set(vec!["a".into()])),
    ];
    for (kind, value) in &cases {
        assert!(conforms(kind, value), "{kind:?} takes {value:?}");
        assert!(conforms(&Kind::Any, value));
        for (other, _) in cases.iter().filter(|(other, _)| other != kind) {
            assert!(!conforms(other, value), "{other:?} does not take {value:?}");
        }
    }
    // A number of any kind is a number, and nothing else is.
    for (kind, value) in &cases[3..11] {
        assert!(conforms(&Kind::Number, value), "{kind:?}");
    }
    assert!(!conforms(&Kind::Number, &"1".into()));
}

#[test]
fn kinds_with_conditions() {
    let record = Value::RecordId(RecordId { table: "user".into(), key: RecordIdKey::Number(1) });
    assert!(conforms(&Kind::Record(vec![]), &record));
    assert!(!conforms(&Kind::Record(vec!["post".into()]), &record));
    let point = Value::Geometry(Geometry::Point(Point { x: 1.0, y: 2.0 }));
    assert!(conforms(&Kind::Geometry(vec![]), &point));
    assert!(!conforms(&Kind::Geometry(vec![GeometryKind::Line]), &point));

    let numbers: Value = vec![1.into(), 2.into(), 3.into()].into();
    assert!(conforms(&Kind::Array(Box::new(Kind::Int), Some(3)), &numbers));
    assert!(!conforms(&Kind::Array(Box::new(Kind::Int), Some(2)), &numbers));
    assert!(!conforms(&Kind::Array(Box::new(Kind::String), None), &numbers));

    let optional = Kind::Either(vec![Kind::None, Kind::String]);
    assert!(conforms(&optional, &Value::None) && conforms(&optional, &"a".into()));
    assert!(!conforms(&optional, &1.into()));

    let colour = Kind::Enum(vec!["white".into(), "black".into()]);
    assert!(conforms(&colour, &"white".into()) && !conforms(&colour, &"red".into()));

    let literal = |literal| Kind::Literal(literal);
    assert!(conforms(&literal(KindLiteral::String("a".into())), &"a".into()));
    assert!(!conforms(&literal(KindLiteral::String("a".into())), &"b".into()));
    assert!(conforms(&literal(KindLiteral::Integer(1)), &1.into()));
    assert!(!conforms(&literal(KindLiteral::Integer(1)), &2.into()));
    let shape = KindLiteral::Object(BTreeMap::from([
        ("city".to_owned(), Kind::String),
        ("zip".to_owned(), Kind::String),
    ]));
    let address = object([("city", "Brussels".into()), ("zip", "1000".into())]);
    assert!(conforms(&literal(shape.clone()), &address));
    let more = object([("city", "B".into()), ("zip", "1".into()), ("country", "BE".into())]);
    assert!(!conforms(&literal(shape), &more));
    let pair = KindLiteral::Array(vec![Kind::Int, Kind::String]);
    assert!(conforms(&literal(pair.clone()), &vec![1.into(), "a".into()].into()));
    assert!(!conforms(&literal(pair), &vec![1.into()].into()));

    // A kind given by name is the kind it names, and one that names itself is no kind.
    let named = BTreeMap::from([
        ("Age".to_owned(), Kind::U8),
        ("Loop".to_owned(), Kind::Named("Loop".to_owned())),
    ]);
    let age = encode(&Value::Number(Number::U8(9))).unwrap();
    let age = Doc::new(&age).unwrap().root();
    assert!(Kind::Named("Age".into()).conforms_with(age, &named));
    assert!(!Kind::Named("Age".into()).conforms(age));
    assert!(!Kind::Named("Loop".into()).conforms_with(age, &named));
    assert!(!Kind::Function(None, None).conforms(age));
}
