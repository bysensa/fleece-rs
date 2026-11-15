//! The keys of values against SurrealDB's own types (`surrealdb-types`, a dev-dependency):
//! its ordered encoding of numbers, and the order it gives its values.

use std::collections::BTreeMap;

use fleece::key::encode_key;
use fleece::{
    BigDecimal, Date, Datetime, Decimal, Duration, F16, Number, Time, Timestamp, Uuid, Value,
};
use proptest::collection::{btree_map, vec};
use proptest::prelude::*;
use surrealdb_types as surreal;

/// The value as SurrealDB holds it.
fn to_surreal(value: &Value) -> surreal::Value {
    match value {
        Value::None => surreal::Value::None,
        Value::Null => surreal::Value::Null,
        Value::Bool(v) => surreal::Value::Bool(*v),
        Value::Number(number) => surreal::Value::Number(match number {
            Number::Int(v) => surreal::Number::Int(*v),
            Number::Float(v) => surreal::Number::Float(*v),
            Number::F32(v) => surreal::Number::F32(*v),
            Number::F16(v) => surreal::Number::F16(half::f16::from_bits(v.0)),
            Number::I8(v) => surreal::Number::I8(*v),
            Number::U8(v) => surreal::Number::U8(*v),
            Number::I16(v) => surreal::Number::I16(*v),
            Number::U16(v) => surreal::Number::U16(*v),
            Number::I32(v) => surreal::Number::I32(*v),
            Number::U32(v) => surreal::Number::U32(*v),
            Number::U64(v) => surreal::Number::U64(*v),
            Number::Decimal(v) => {
                surreal::Number::Decimal(surreal::Decimal::deserialize(v.to_bytes()))
            }
            Number::BigDecimal(v) => {
                surreal::Number::BigDecimal(Box::new(v.to_string().parse().unwrap()))
            }
        }),
        Value::String(v) => surreal::Value::String(v.clone()),
        Value::Bytes(v) => surreal::Value::Bytes(surreal::Bytes::from(v.clone())),
        Value::Duration(v) => surreal::Value::Duration(surreal::Duration::new(v.secs, v.nanos)),
        Value::Datetime(v) => {
            surreal::Value::Datetime(surreal::Datetime::try_from((v.secs, v.nanos)).unwrap())
        }
        Value::Uuid(v) => surreal::Value::Uuid(surreal::Uuid::try_from(v.to_string()).unwrap()),
        Value::Table(v) => surreal::Value::Table(surreal::Table::new(v.clone())),
        Value::Date(v) => surreal::Value::Date(surreal::Date::from_days(v.days).unwrap()),
        Value::Time(v) => surreal::Value::Time(surreal::Time::from_nanos(v.nanos).unwrap()),
        Value::Timestamp(v) => {
            surreal::Value::Timestamp(surreal::Timestamp::from_parts(v.secs, v.nanos).unwrap())
        }
        Value::Array(items) => {
            let items: Vec<surreal::Value> = items.iter().map(to_surreal).collect();
            surreal::Value::Array(surreal::Array::from(items))
        }
        Value::Object(fields) => {
            let fields: BTreeMap<String, surreal::Value> =
                fields.iter().map(|(key, value)| (key.clone(), to_surreal(value))).collect();
            surreal::Value::Object(surreal::Object::from(fields))
        }
        other => panic!("not in this test: {other:?}"),
    }
}

fn number() -> impl Strategy<Value = Number> {
    let decimal = |mantissa: u128, scale: u8, negative: bool| {
        Number::Decimal(Decimal::new(mantissa, scale, negative).unwrap())
    };
    prop_oneof![
        any::<i64>().prop_map(Number::Int),
        (-300i64..300).prop_map(Number::Int),
        any::<u64>().prop_map(Number::U64),
        any::<i8>().prop_map(Number::I8),
        any::<u32>().prop_map(Number::U32),
        any::<f64>().prop_map(Number::Float),
        (-2400i32..2400).prop_map(|eighths| Number::Float(f64::from(eighths) / 8.0)),
        any::<f32>().prop_map(Number::F32),
        any::<u16>().prop_map(|bits| Number::F16(F16(bits))),
        (0..1u128 << 96, 0..=28u8, any::<bool>()).prop_map(move |(m, s, n)| decimal(m, s, n)),
        (0..300_000u128, 0..5u8, any::<bool>()).prop_map(move |(m, s, n)| decimal(m, s, n)),
        "-?[0-9]{1,30}(\\.[0-9]{1,12})?"
            .prop_map(|text| Number::BigDecimal(text.parse::<BigDecimal>().unwrap())),
    ]
}

/// Values of the types whose order SurrealDB defines and this crate claims to follow.
fn value() -> impl Strategy<Value = Value> {
    // Within what `chrono` holds, which SurrealDB's dates and times are made of.
    let secs = -8_000_000_000_000i64..8_000_000_000_000;
    let nanos = 0..1_000_000_000u32;
    let leaf = prop_oneof![
        Just(Value::None),
        Just(Value::Null),
        any::<bool>().prop_map(Value::Bool),
        number().prop_map(Value::Number),
        number().prop_map(Value::Number),
        "[a-c\\x00]{0,4}".prop_map(Value::String),
        vec(0..3u8, 0..4).prop_map(Value::Bytes),
        (0..5u64, nanos.clone())
            .prop_map(|(secs, nanos)| Value::Duration(Duration { secs, nanos })),
        (secs.clone(), nanos.clone())
            .prop_map(|(secs, nanos)| Value::Datetime(Datetime { secs, nanos })),
        any::<[u8; 16]>().prop_map(|bytes| Value::Uuid(Uuid(bytes))),
        "[a-c]{1,3}".prop_map(Value::Table),
        (-90_000_000i32..90_000_000).prop_map(|days| Value::Date(Date { days })),
        (0..=Time::MAX_NANOS).prop_map(|nanos| Value::Time(Time { nanos })),
        (secs, nanos).prop_map(|(secs, nanos)| Value::Timestamp(Timestamp { secs, nanos })),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            vec(inner.clone(), 0..4).prop_map(Value::Array),
            btree_map("[a-c\\x00]{0,2}", inner, 0..4).prop_map(Value::Object),
        ]
    })
}

proptest! {
    #[test]
    fn the_key_of_a_number_is_surrealdbs_ordered_encoding(
        text in "-?[0-9]{1,40}(\\.[0-9]{1,20})?",
    ) {
        let reference: surreal::BigDecimal = text.parse().unwrap();
        let mut expected = Vec::new();
        reference.encode_ordered(&mut expected);
        let ours = encode_key(&Value::Number(Number::BigDecimal(text.parse().unwrap())));
        // After the byte that says the value is a number.
        prop_assert_eq!(&ours[1..], &expected[..]);
    }

    #[test]
    fn keys_sort_as_surrealdb_sorts_its_values(a in value(), b in value()) {
        let expected = to_surreal(&a).cmp(&to_surreal(&b));
        prop_assert_eq!(encode_key(&a).cmp(&encode_key(&b)), expected, "{:?} {:?}", a, b);
    }
}
