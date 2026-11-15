//! The bytes of every form of value, written down once.
//!
//! Documents are kept in stores for longer than any version of this crate lives. A change
//! that makes one of these tests fail changes what is on disk: it needs a new
//! [`fleece::FORMAT_VERSION`] and a way to read what the old one wrote, not a new expectation
//! here.

use std::ops::Bound;

use fleece::{
    BigDecimal, Date, Datetime, Decimal, Duration, F16, File, Geometry, Interval, Number, Object,
    Point, Range, RecordId, RecordIdKey, Time, Timestamp, Uuid, Value, Vector, decode, encode,
};

fn hex(text: &str) -> Vec<u8> {
    text.split_whitespace().map(|byte| u8::from_str_radix(byte, 16).unwrap()).collect()
}

/// The value is written as these bytes, and these bytes are read as the value.
#[track_caller]
fn frozen(value: Value, bytes: &str) {
    let written: Vec<String> =
        encode(&value).unwrap().iter().map(|byte| format!("{byte:02x}")).collect();
    assert_eq!(written.join(" "), bytes, "{value:?}");
    assert_eq!(decode(&hex(bytes)).unwrap(), value);
}

fn number(number: Number) -> Value {
    Value::Number(number)
}

#[test]
fn the_format_has_a_version() {
    assert_eq!(fleece::FORMAT_VERSION, 1);
}

#[test]
fn standard_values() {
    frozen(Value::Null, "30 00");
    frozen(Value::Bool(false), "34 00");
    frozen(Value::Bool(true), "38 00");
    frozen(Value::from(0), "00 00");
    frozen(Value::from(-1), "0f ff");
    frozen(Value::from(2047), "07 ff");
    frozen(Value::from(2048), "11 00 08 00 80 02");
    frozen(Value::from(i64::MIN), "17 00 00 00 00 00 00 00 80 00 80 05");
    frozen(Value::from(1.5), "24 00 00 00 c0 3f 80 03");
    frozen(Value::from(0.1), "28 00 9a 99 99 99 99 99 b9 3f 80 05");
    frozen(number(Number::F32(1.5)), "20 00 00 00 c0 3f 80 03");
    frozen(Value::from(""), "40 00");
    frozen(Value::from("a"), "41 61");
    frozen(Value::from("abc"), "43 61 62 63 80 02");
    frozen(
        Value::from("abcdefghijklmno"),
        "4f 0f 61 62 63 64 65 66 67 68 69 6a 6b 6c 6d 6e 6f 00 80 09",
    );
    frozen(Value::Bytes(vec![1, 2, 3]), "53 01 02 03 80 02");
    frozen(Value::Array(vec![]), "60 00");
    frozen(Value::Array(vec![1.into(), "abc".into()]), "43 61 62 63 60 02 00 01 80 04 80 03");
    frozen(Value::Object(Object::new()), "70 00");
    frozen(Value::Object(Object::from([("a".to_owned(), 1.into())])), "70 01 41 61 00 01 80 03");
}

#[test]
fn extended_numbers() {
    frozen(Value::None, "31 00");
    frozen(number(Number::I8(-1)), "31 01 0f ff 80 02");
    frozen(number(Number::U8(7)), "31 02 00 07 80 02");
    frozen(number(Number::I16(-300)), "31 03 0e d4 80 02");
    frozen(number(Number::U16(u16::MAX)), "31 04 12 ff ff 00 80 03");
    frozen(number(Number::I32(i32::MIN)), "31 05 13 00 00 00 80 00 80 04");
    frozen(number(Number::U32(u32::MAX)), "31 06 14 ff ff ff ff 00 80 04");
    frozen(number(Number::U64(u64::MAX)), "31 07 1f ff ff ff ff ff ff ff ff 00 80 06");
    frozen(number(Number::F16(F16(0x3E00))), "31 08 11 00 3e 00 80 03");
    frozen(
        number(Number::Decimal(Decimal::new(150, 2, false).unwrap())),
        "31 09 5f 10 00 00 02 00 96 00 00 00 00 00 00 00 00 00 00 00 80 0a",
    );
    frozen(
        number(Number::BigDecimal("-1.50".parse::<BigDecimal>().unwrap())),
        "52 6a ff 00 31 0a 60 02 00 02 80 05 80 04",
    );
}

#[test]
fn vectors() {
    // The type of the elements, then the elements; for bits the unused bits of the last
    // byte come between.
    frozen(
        Value::Vector(Vector::F32(vec![1.0, -2.0])),
        "31 19 59 00 00 00 80 3f 00 00 00 c0 80 06",
    );
    frozen(Value::Vector(Vector::F64(vec![1.0])), "31 19 59 01 00 00 00 00 00 00 f0 3f 80 06");
    frozen(Value::Vector(Vector::F16(vec![F16(0x3E00)])), "31 19 53 02 00 3e 80 03");
    frozen(Value::Vector(Vector::I8(vec![-1, 127])), "31 19 53 03 ff 7f 80 03");
    frozen(Value::Vector(Vector::Bits { bytes: vec![0xA1], len: 8 }), "31 19 53 04 00 a1 80 03");
    frozen(
        Value::Vector(Vector::Bits { bytes: vec![0xA1, 0x03], len: 10 }),
        "31 19 54 04 06 a1 03 00 80 04",
    );
    frozen(Value::Vector(Vector::F32(vec![])), "31 19 51 00 80 02");
}

#[test]
fn extended_times() {
    frozen(
        Value::Duration(Duration { secs: 1, nanos: 500_000_000 }),
        "31 0b 5c 01 00 00 00 00 00 00 00 00 65 cd 1d 00 80 08",
    );
    frozen(
        Value::Datetime(Datetime { secs: -1, nanos: 0 }),
        "31 0c 5c ff ff ff ff ff ff ff ff 00 00 00 00 00 80 08",
    );
    frozen(Value::Date(Date { days: 20_772 }), "31 0e 11 24 51 00 80 03");
    frozen(Value::Time(Time { nanos: 1 }), "31 0f 00 01 80 02");
    frozen(
        Value::Timestamp(Timestamp { secs: 0, nanos: 1 }),
        "31 10 5c 00 00 00 00 00 00 00 00 01 00 00 00 00 80 08",
    );
    frozen(
        Value::Interval(Interval { months: 1, days: -1, micros: 2 }),
        "31 11 5f 10 01 00 00 00 ff ff ff ff 02 00 00 00 00 00 00 00 80 0a",
    );
}

#[test]
fn extended_values() {
    let uuid = Uuid([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]);
    frozen(Value::Uuid(uuid), "31 0d 5f 10 01 02 03 04 05 06 07 08 09 0a 0b 0c 0d 0e 0f 10 80 0a");
    frozen(Value::Table("t".into()), "31 12 41 74 80 02");
    frozen(
        Value::RecordId(RecordId { table: "user".into(), key: RecordIdKey::Number(7) }),
        "44 75 73 65 72 00 31 13 60 02 80 05 00 07 80 04",
    );
    frozen(
        Value::File(File { bucket: "b".into(), key: "/k".into() }),
        "42 2f 6b 00 31 14 60 02 41 62 80 05 80 04",
    );
    frozen(
        Value::Range(Box::new(Range { start: Bound::Included(1.into()), end: Bound::Unbounded })),
        "31 15 60 03 00 01 00 01 30 00 80 05",
    );
    frozen(Value::Regex("a+".into()), "31 16 42 61 2b 00 80 03");
    frozen(Value::Set(vec![1.into()]), "31 17 60 01 00 01 80 03");
    frozen(
        Value::Geometry(Geometry::Point(Point { x: 1.0, y: 2.0 })),
        "5f 10 00 00 00 00 00 00 f0 3f 00 00 00 00 00 00 00 40 \
         31 18 60 02 00 00 80 0c 80 04",
    );
}
