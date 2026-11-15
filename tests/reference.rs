//! The layouts of the numeric types against the crates SurrealDB holds them with.

use std::cmp::Ordering;

use fleece::key::encode_key;
use fleece::{
    BigDecimal, Date, Datetime, Decimal, Duration, F16, Number, Object, Time, Timestamp, Uuid,
    Value,
};
use num_bigint::BigInt;
use proptest::collection::vec;
use proptest::prelude::*;

/// A finite number as `mantissa * 2^twos * 10^-scale`, exactly.
fn exact(number: &Number) -> (BigInt, i64, u32) {
    let double = |value: f64| {
        let bits = value.to_bits();
        let (biased, fraction) = ((bits >> 52 & 0x7FF) as i64, bits & ((1 << 52) - 1));
        let (mantissa, twos) = match biased {
            0 => (fraction, -1074),
            _ => (fraction | 1 << 52, biased - 1075),
        };
        let mantissa = BigInt::from(mantissa);
        (if value.is_sign_negative() { -mantissa } else { mantissa }, twos, 0)
    };
    match number {
        Number::Int(v) => (BigInt::from(*v), 0, 0),
        Number::I8(v) => (BigInt::from(*v), 0, 0),
        Number::I16(v) => (BigInt::from(*v), 0, 0),
        Number::I32(v) => (BigInt::from(*v), 0, 0),
        Number::U8(v) => (BigInt::from(*v), 0, 0),
        Number::U16(v) => (BigInt::from(*v), 0, 0),
        Number::U32(v) => (BigInt::from(*v), 0, 0),
        Number::U64(v) => (BigInt::from(*v), 0, 0),
        Number::Float(v) => double(*v),
        Number::F32(v) => double(f64::from(*v)),
        Number::F16(v) => double(v.to_f64()),
        Number::Decimal(v) => {
            let mantissa = BigInt::from(v.mantissa());
            (if v.is_negative() { -mantissa } else { mantissa }, 0, u32::from(v.scale()))
        }
        Number::BigDecimal(v) => (BigInt::from_signed_bytes_le(v.coefficient()), 0, v.scale()),
    }
}

/// The order of two finite numbers by what they are worth, with whole numbers only.
fn compare(a: &Number, b: &Number) -> Ordering {
    let ((a, a_twos, a_scale), (b, b_twos, b_scale)) = (exact(a), exact(b));
    let (twos, scale) = (a_twos.min(b_twos), a_scale.max(b_scale));
    let whole = |mantissa: BigInt, own_twos: i64, own_scale: u32| {
        (mantissa << (own_twos - twos) as usize) * BigInt::from(10).pow(scale - own_scale)
    };
    whole(a, a_twos, a_scale).cmp(&whole(b, b_twos, b_scale))
}

fn finite_number() -> impl Strategy<Value = Number> {
    let decimal = |mantissa: u128, scale: u8, negative: bool| {
        Number::Decimal(Decimal::new(mantissa, scale, negative).unwrap())
    };
    prop_oneof![
        any::<i64>().prop_map(Number::Int),
        (-300i64..300).prop_map(Number::Int),
        any::<u64>().prop_map(Number::U64),
        any::<i8>().prop_map(Number::I8),
        any::<u16>().prop_map(Number::U16),
        any::<f64>().prop_filter("finite", |v| v.is_finite()).prop_map(Number::Float),
        // Floats that whole numbers and short decimals are equal to.
        (-2400i32..2400).prop_map(|eighths| Number::Float(f64::from(eighths) / 8.0)),
        any::<f32>().prop_filter("finite", |v| v.is_finite()).prop_map(Number::F32),
        any::<u16>()
            .prop_filter("finite", |bits| F16(*bits).to_f64().is_finite())
            .prop_map(|bits| Number::F16(F16(bits))),
        (0..1u128 << 96, 0..=28u8, any::<bool>()).prop_map(move |(m, s, n)| decimal(m, s, n)),
        (0..300_000u128, 0..5u8, any::<bool>()).prop_map(move |(m, s, n)| decimal(m, s, n)),
        "-?[0-9]{1,30}(\\.[0-9]{1,12})?"
            .prop_map(|text| Number::BigDecimal(text.parse::<BigDecimal>().unwrap())),
    ]
}

fn key(number: Number) -> Vec<u8> {
    encode_key(&Value::Number(number))
}

#[test]
fn keys_of_numbers_that_are_not_finite() {
    let order = [
        key(Number::Float(f64::NEG_INFINITY)),
        key(Number::Float(f64::MIN)),
        key(Number::Int(i64::MIN)),
        key(Number::Float(-f64::from_bits(1))),
        key(Number::Int(0)),
        key(Number::Float(f64::from_bits(1))),
        key(Number::U64(u64::MAX)),
        key(Number::Float(f64::MAX)),
        key(Number::Float(f64::INFINITY)),
        key(Number::Float(f64::NAN)),
    ];
    assert!(order.is_sorted_by(|a, b| a < b), "{order:?}");
    assert_eq!(key(Number::F32(f32::INFINITY)), key(Number::Float(f64::INFINITY)));
}

#[test]
fn keys_of_values_of_different_types() {
    let order = [
        Value::None,
        Value::Null,
        Value::Bool(false),
        Value::Bool(true),
        Value::from(-1),
        Value::from(0.5),
        Value::from(""),
        Value::from("a"),
        Value::from("a\0"),
        Value::from("ab"),
        Value::from("b"),
        Value::Duration(Duration { secs: 1, nanos: 0 }),
        Value::Datetime(Datetime { secs: -1, nanos: 999_999_999 }),
        Value::Datetime(Datetime { secs: 0, nanos: 0 }),
        Value::Uuid(Uuid([0; 16])),
        Value::Array(vec![]),
        Value::Array(vec![1.into()]),
        Value::Array(vec![1.into(), Value::None]),
        Value::Array(vec![2.into()]),
        Value::Set(vec![]),
        Value::Object(Object::new()),
        Value::Object(Object::from([("a".to_owned(), 1.into())])),
        Value::Object(Object::from([("a".to_owned(), 1.into()), ("b".to_owned(), 0.into())])),
        Value::Object(Object::from([("a".to_owned(), 2.into())])),
        Value::Object(Object::from([("b".to_owned(), 0.into())])),
        Value::Bytes(vec![0]),
        Value::Table("t".into()),
        Value::Regex("r".into()),
        Value::Date(Date { days: -1 }),
        Value::Date(Date { days: 0 }),
        Value::Time(Time { nanos: 0 }),
        Value::Timestamp(Timestamp { secs: 0, nanos: 0 }),
    ];
    let keys: Vec<Vec<u8>> = order.iter().map(encode_key).collect();
    for (pair, values) in keys.windows(2).zip(order.windows(2)) {
        assert!(pair[0] < pair[1], "{:?} sorts before {:?}", values[0], values[1]);
    }
}

proptest! {
    #[test]
    fn keys_of_numbers_sort_as_the_numbers(a in finite_number(), b in finite_number()) {
        prop_assert_eq!(key(a.clone()).cmp(&key(b.clone())), compare(&a, &b), "{:?} {:?}", a, b);
    }

    #[test]
    fn keys_of_bytes_and_lists_sort_as_they_do(
        a in vec(vec(any::<u8>(), 0..6), 0..4),
        b in vec(vec(any::<u8>(), 0..6), 0..4),
    ) {
        let list = |items: &Vec<Vec<u8>>| {
            Value::Array(items.iter().cloned().map(Value::Bytes).collect())
        };
        prop_assert_eq!(encode_key(&list(&a)).cmp(&encode_key(&list(&b))), a.cmp(&b));
        let (first, second) = (a.concat(), b.concat());
        let (left, right) = (Value::Bytes(first.clone()), Value::Bytes(second.clone()));
        prop_assert_eq!(encode_key(&left).cmp(&encode_key(&right)), first.cmp(&second));
    }
}

#[test]
fn f16_is_the_f16_of_half() {
    for bits in 0..=u16::MAX {
        let reference = half::f16::from_bits(bits).to_f64();
        let ours = F16(bits).to_f64();
        // A NaN is a NaN on both sides; which of the NaNs of a double it becomes is not
        // part of the value, and the 16 bits themselves are what is stored.
        if reference.is_nan() {
            assert!(ours.is_nan(), "{bits:#06x}");
        } else {
            assert_eq!(ours.to_bits(), reference.to_bits(), "{bits:#06x}");
        }
    }
}

proptest! {
    #[test]
    fn decimal_is_the_decimal_of_rust_decimal(
        mantissa in 0..1u128 << 96,
        scale in 0..=28u8,
        negative: bool,
    ) {
        let mut reference =
            rust_decimal::Decimal::from_i128_with_scale(mantissa as i128, u32::from(scale));
        reference.set_sign_negative(negative);
        let ours = Decimal::new(mantissa, scale, negative).unwrap();
        // The 16 bytes are those of `serialize`, each way.
        prop_assert_eq!(ours.to_bytes(), reference.serialize());
        prop_assert_eq!(Decimal::from_bytes(reference.serialize()), Some(ours));
        prop_assert_eq!(rust_decimal::Decimal::deserialize(ours.to_bytes()), reference);
        // And the text of one is read by the other.
        if mantissa != 0 || !negative {
            prop_assert_eq!(ours.to_string(), reference.to_string());
        }
        prop_assert_eq!(ours.to_string().parse::<Decimal>(), Ok(ours));
        prop_assert_eq!(ours.to_string().parse::<rust_decimal::Decimal>(), Ok(reference));
    }

    #[test]
    fn a_double_is_an_f16_when_half_says_so(value: f64) {
        let exact = half::f16::from_f64(value).to_f64().to_bits() == value.to_bits();
        prop_assert_eq!(F16::from_f64(value).is_some(), exact || value.is_nan());
    }

    #[test]
    fn the_coefficient_is_the_bytes_of_a_bigint(
        bytes in vec(any::<u8>(), 0..80),
        scale in 0..40u32,
    ) {
        let reference = BigInt::from_signed_bytes_le(&bytes);
        let ours = BigDecimal::new(&bytes, scale).unwrap();
        prop_assert_eq!(ours.coefficient(), &reference.to_signed_bytes_le()[..]);
        prop_assert_eq!(ours.is_negative(), reference.sign() == num_bigint::Sign::Minus);

        // The digits are the digits of the integer, with the point `scale` from the end.
        let text = ours.to_string();
        prop_assert_eq!(text.replace('.', "").parse::<BigInt>().unwrap(), reference.clone());
        let digits_after_point = text.split_once('.').map_or(0, |(_, fraction)| fraction.len());
        prop_assert_eq!(digits_after_point, scale as usize);
        // And the text reads back as the same bytes, but for the sign of a zero.
        prop_assert_eq!(text.parse::<BigDecimal>(), Ok(ours));
    }
}
