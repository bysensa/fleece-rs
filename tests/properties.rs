//! Properties that hold for every value and for any bytes.

use std::ops::Bound;

use fleece::delta::{append_delta, apply, diff};
use fleece::{
    BigDecimal, Date, Datetime, Decimal, Doc, Duration, Encoder, F16, File, Geometry, Interval,
    KeyPath, Number, Point, Polygon, Range, RecordId, RecordIdKey, RecordIdKeyRange, SharedKeys,
    Time, Timestamp, Uuid, Value, decode, dump, encode,
};
use proptest::collection::{btree_map, vec};
use proptest::num::{f32, f64};
use proptest::prelude::*;

/// Every float but NaN, which is not equal to itself.
fn double() -> impl Strategy<Value = f64> {
    f64::POSITIVE | f64::NEGATIVE | f64::NORMAL | f64::SUBNORMAL | f64::ZERO | f64::INFINITE
}

fn number() -> impl Strategy<Value = Number> {
    let single =
        f32::POSITIVE | f32::NEGATIVE | f32::NORMAL | f32::SUBNORMAL | f32::ZERO | f32::INFINITE;
    let half = any::<u16>().prop_filter("NaN", |bits| !F16(*bits).to_f64().is_nan());
    let decimal = (0..1u128 << 96, 0..=Decimal::MAX_SCALE, any::<bool>());
    prop_oneof![
        any::<i64>().prop_map(Number::Int),
        double().prop_map(Number::Float),
        decimal.prop_map(|(mantissa, scale, negative)| {
            Number::Decimal(Decimal::new(mantissa, scale, negative).unwrap())
        }),
        any::<i8>().prop_map(Number::I8),
        any::<u8>().prop_map(Number::U8),
        any::<i16>().prop_map(Number::I16),
        any::<u16>().prop_map(Number::U16),
        any::<i32>().prop_map(Number::I32),
        any::<u32>().prop_map(Number::U32),
        any::<u64>().prop_map(Number::U64),
        half.prop_map(|bits| Number::F16(F16(bits))),
        single.prop_map(Number::F32),
        "-?[0-9]{1,40}(\\.[0-9]{1,20})?"
            .prop_map(|text| Number::BigDecimal(text.parse::<BigDecimal>().unwrap())),
    ]
}

fn points() -> impl Strategy<Value = Vec<Point>> {
    vec((-180.0..180.0, -90.0..90.0).prop_map(|(x, y)| Point { x, y }), 0..5)
}

fn polygon() -> impl Strategy<Value = Polygon> {
    (points(), vec(points(), 0..3))
        .prop_map(|(exterior, interiors)| Polygon { exterior, interiors })
}

fn geometry() -> impl Strategy<Value = Geometry> {
    let shape = prop_oneof![
        points().prop_map(|points| Geometry::Point(points.first().copied().unwrap_or_default())),
        points().prop_map(Geometry::Line),
        polygon().prop_map(Geometry::Polygon),
        points().prop_map(Geometry::MultiPoint),
        vec(points(), 0..3).prop_map(Geometry::MultiLine),
        vec(polygon(), 0..3).prop_map(Geometry::MultiPolygon),
    ];
    shape.prop_recursive(2, 8, 3, |inner| vec(inner, 0..3).prop_map(Geometry::Collection))
}

fn key() -> impl Strategy<Value = String> {
    // Short names, any text, and names a `$` starts, which JSON has to escape.
    prop_oneof!["[a-z_]{1,10}", ".{0,6}", "\\$\\$?[a-z]{1,4}"]
}

fn bound<T: std::fmt::Debug + Clone>(
    inner: impl Strategy<Value = T> + Clone,
) -> impl Strategy<Value = Bound<T>> {
    prop_oneof![
        Just(Bound::Unbounded),
        inner.clone().prop_map(Bound::Included),
        inner.prop_map(Bound::Excluded),
    ]
}

fn record_key(inner: impl Strategy<Value = Value> + Clone) -> impl Strategy<Value = RecordIdKey> {
    let simple = prop_oneof![
        any::<i64>().prop_map(RecordIdKey::Number),
        ".{0,12}".prop_map(RecordIdKey::String),
        any::<[u8; 16]>().prop_map(|bytes| RecordIdKey::Uuid(Uuid(bytes))),
    ];
    prop_oneof![
        simple.clone(),
        vec(inner.clone(), 0..4).prop_map(RecordIdKey::Array),
        btree_map(key(), inner, 0..4).prop_map(RecordIdKey::Object),
        (bound(simple.clone()), bound(simple)).prop_map(|(start, end)| {
            RecordIdKey::Range(Box::new(RecordIdKeyRange { start, end }))
        }),
    ]
}

fn value() -> impl Strategy<Value = Value> {
    let nanos = 0..1_000_000_000u32;
    let leaf = prop_oneof![
        Just(Value::None),
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        number().prop_map(Value::Number),
        ".{0,20}".prop_map(Value::String),
        vec(any::<u8>(), 0..40).prop_map(Value::Bytes),
        (any::<u64>(), nanos.clone())
            .prop_map(|(secs, nanos)| Value::Duration(Duration { secs, nanos })),
        (any::<i64>(), nanos.clone())
            .prop_map(|(secs, nanos)| Value::Datetime(Datetime { secs, nanos })),
        (any::<i64>(), nanos).prop_map(|(secs, nanos)| Value::Timestamp(Timestamp { secs, nanos })),
        any::<[u8; 16]>().prop_map(|bytes| Value::Uuid(Uuid(bytes))),
        any::<i32>().prop_map(|days| Value::Date(Date { days })),
        (0..=Time::MAX_NANOS).prop_map(|nanos| Value::Time(Time { nanos })),
        (any::<i32>(), any::<i32>(), any::<i64>())
            .prop_map(|(months, days, micros)| Value::Interval(Interval { months, days, micros })),
        "[a-z]{1,8}".prop_map(Value::Table),
        ".{0,10}".prop_map(Value::Regex),
        ("[a-z]{0,6}", "/[a-z]{0,6}").prop_map(|(bucket, key)| Value::File(File { bucket, key })),
        geometry().prop_map(Value::Geometry),
    ];
    leaf.prop_recursive(4, 48, 6, |inner| {
        prop_oneof![
            vec(inner.clone(), 0..6).prop_map(Value::Array),
            btree_map(key(), inner.clone(), 0..6).prop_map(Value::Object),
            vec(inner.clone(), 0..4).prop_map(Value::Set),
            (bound(inner.clone()), bound(inner.clone()))
                .prop_map(|(start, end)| Value::Range(Box::new(Range { start, end }))),
            ("[a-z]{1,8}", record_key(inner))
                .prop_map(|(table, key)| Value::RecordId(RecordId { table, key })),
        ]
    })
}

/// Reads bytes every way the crate offers. None of them may panic.
fn read_everything(bytes: &[u8]) {
    if let Ok(doc) = Doc::new(bytes) {
        let _ = dump(&doc);
        let _ = Value::from_ref(doc.root());
    }
    let doc = Doc::trusted(bytes);
    let _ = Value::from_ref(doc.root());
    let _ = dump(&doc);
    let path: KeyPath = "a[0].b[-1]".parse().unwrap();
    let _ = path.eval(doc.root());
}

proptest! {
    #[test]
    fn a_value_reads_back_as_written(value in value()) {
        let bytes = encode(&value).unwrap();
        prop_assert_eq!(bytes.len() % 2, 0);
        prop_assert_eq!(decode(&bytes).unwrap(), value);
    }

    #[test]
    fn a_value_reads_back_with_shared_keys(value in value()) {
        let mut shared = SharedKeys::new();
        let mut encoder = Encoder::new().with_shared_keys(&mut shared);
        encoder.write_value(&value).unwrap();
        let bytes = encoder.finish().unwrap();
        let doc = Doc::trusted(&bytes).with_shared_keys(&shared);
        prop_assert_eq!(doc.validate(), Ok(()));
        prop_assert_eq!(Value::from_ref(doc.root()).unwrap(), value);
    }

    #[test]
    fn any_bytes_are_read_without_a_panic(bytes in vec(any::<u8>(), 0..200)) {
        read_everything(&bytes);
    }

    #[test]
    fn a_damaged_document_is_read_without_a_panic(
        value in value(),
        damage in vec((any::<prop::sample::Index>(), any::<u8>()), 1..4),
    ) {
        let mut bytes = encode(&value).unwrap();
        for (at, byte) in damage {
            let at = at.index(bytes.len());
            bytes[at] = byte;
        }
        read_everything(&bytes);
    }

    #[test]
    fn a_structural_delta_turns_old_into_new(old in value(), new in value()) {
        match diff(&old, &new) {
            None => prop_assert_eq!(&old, &new),
            Some(delta) => {
                prop_assert_eq!(apply(&old, &delta), Ok(new));
                // The delta is a value like any other.
                prop_assert_eq!(decode(&encode(&delta).unwrap()).unwrap(), delta);
            }
        }
    }

    #[test]
    fn an_appended_delta_makes_the_new_document(old in value(), new in value()) {
        let base = encode(&old).unwrap();
        let delta = append_delta(&base, &new).unwrap();
        let appended = [&base[..], &delta[..]].concat();
        prop_assert_eq!(decode(&appended).unwrap(), new.clone());

        // The same delta kept apart from its base, its pointers marked as leaving it.
        let mut encoder = Encoder::new().with_base(&base, true).unwrap();
        encoder.write_changed(Doc::trusted(&base).root(), &new).unwrap();
        let apart = encoder.finish().unwrap();
        let doc = Doc::trusted(&apart).with_base(&base);
        prop_assert_eq!(doc.validate(), Ok(()));
        prop_assert_eq!(Value::from_ref(doc.root()).unwrap(), new);
    }
}

/// The fields a change sets, or deletes when there is no value.
type Change = Vec<(String, Option<Value>)>;

/// A record of many fields, and a few changes of it: fields set, added and deleted.
fn record_and_changes() -> impl Strategy<Value = (fleece::Object, Vec<Change>)> {
    let record = btree_map("[a-z]{1,3}", value(), 20..60);
    let change = vec(("[a-z]{1,3}", proptest::option::weighted(0.7, value())), 1..4);
    (record, vec(change, 1..12))
}

proptest! {
    #[test]
    fn a_document_that_took_many_deltas_reads_as_the_last_value_and_compacts(
        (mut record, changes) in record_and_changes(),
        shared_keys: bool,
    ) {
        fn encoder(shared: Option<&mut SharedKeys>) -> Encoder<'_> {
            match shared {
                Some(shared) => Encoder::new().with_shared_keys(shared),
                None => Encoder::new(),
            }
        }
        let mut shared = SharedKeys::new();
        let mut encoder_of_base = encoder(shared_keys.then_some(&mut shared));
        encoder_of_base.write_value(&Value::Object(record.clone())).unwrap();
        let mut bytes = encoder_of_base.finish().unwrap();

        for change in changes {
            for (key, value) in change {
                match value {
                    Some(value) => record.insert(key, value),
                    None => record.remove(&key),
                };
            }
            let new = Value::Object(record.clone());
            let table = shared.clone();
            let old = Doc::trusted(&bytes).with_shared_keys(&table).root();
            let mut delta =
                encoder(shared_keys.then_some(&mut shared)).with_base(&bytes, false).unwrap();
            delta.write_changed(old, &new).unwrap();
            bytes.extend(delta.finish().unwrap());

            let doc = Doc::trusted(&bytes).with_shared_keys(&shared);
            prop_assert_eq!(doc.validate(), Ok(()));
            prop_assert_eq!(Value::from_ref(doc.root()).unwrap(), new);
            // Every field is found by its key, through the dictionaries it inherits from.
            let dict = doc.root().as_dict().unwrap();
            prop_assert_eq!(dict.len(), record.len());
            for (key, value) in &record {
                let found = dict.get(key).map(Value::from_ref);
                prop_assert_eq!(found, Some(Ok(value.clone())));
            }
        }

        let doc = Doc::trusted(&bytes).with_shared_keys(&shared);
        let live = doc.live_bytes();
        prop_assert!(live <= bytes.len());
        let compacted = fleece::compact(&doc).unwrap();
        let fresh = Doc::trusted(&compacted).with_shared_keys(&shared);
        prop_assert_eq!(fresh.validate(), Ok(()));
        prop_assert_eq!(Value::from_ref(fresh.root()).unwrap(), Value::Object(record));
        // Nothing is left in a compacted document but what its root leads to.
        prop_assert_eq!(fresh.live_bytes(), compacted.len());
        prop_assert!(compacted.len() <= bytes.len());
    }
}

#[cfg(feature = "json")]
proptest! {
    #[test]
    fn a_value_reads_back_from_json(value in value()) {
        let json = fleece::json::to_json(&value);
        prop_assert_eq!(fleece::json::from_json(&json), Ok(value.clone()));
        // And through text and bytes.
        let text = fleece::json::fleece_to_json(&encode(&value).unwrap()).unwrap();
        let bytes = fleece::json::json_to_fleece(&text).unwrap();
        prop_assert_eq!(decode(&bytes).unwrap(), value);
    }
}

#[test]
fn a_small_change_makes_a_small_delta() {
    let field = |i: usize| (format!("field{i:03}"), Value::from(format!("the value of {i}")));
    let mut fields: fleece::Object = (0..200).map(field).collect();
    let inner = Value::Object(fields.clone());
    fields.insert("inner".to_owned(), inner);
    let old = Value::Object(fields.clone());
    let base = encode(&old).unwrap();

    let Some(Value::Object(inner)) = fields.get_mut("inner") else { panic!() };
    inner.insert("field007".to_owned(), Value::from("changed"));
    let new = Value::Object(fields);
    let delta = append_delta(&base, &new).unwrap();
    // Each of the two dictionaries on the way to the change is written as the one entry
    // that differs over the old dictionary: the changed string, 2 dictionaries of 2 entries
    // and the pointer to the root.
    assert_eq!(delta.len(), 8 + 10 + 10 + 2, "of {}", base.len());
    let appended = [base, delta].concat();
    assert_eq!(decode(&appended).unwrap(), new);
    let doc = Doc::new(&appended).unwrap();
    let inner = doc.root().as_dict().unwrap().get("inner").unwrap().as_dict().unwrap();
    assert_eq!(inner.len(), 200);
    assert_eq!(inner.get("field007").unwrap().as_str(), Some("changed"));
    assert_eq!(inner.get("field008").unwrap().as_str(), Some("the value of 8"));

    // What the delta replaced is still in the bytes, and compaction takes it out.
    assert!(doc.live_bytes() < appended.len());
    let compacted = fleece::compact(&doc).unwrap();
    assert_eq!(decode(&compacted).unwrap(), new);
    assert_eq!(compacted.len(), encode(&new).unwrap().len());

    let change = diff(&old, &new).unwrap();
    let inner = fleece::Object::from([("field007".to_owned(), Value::from("changed"))]);
    let expected = fleece::Object::from([("inner".to_owned(), Value::Object(inner))]);
    assert_eq!(change, Value::Object(expected));
}
