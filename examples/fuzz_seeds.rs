//! Writes the inputs the fuzz targets of `fuzz/` start from: a document of every kind of
//! value, laid out as each target takes its input.
//!
//! ```text
//! cargo run --example fuzz_seeds            # into fuzz/corpus
//! cargo run --example fuzz_seeds -- DIR
//! ```

use std::ops::Bound;
use std::path::Path;

use fleece::json::to_json;
use fleece::{
    BigDecimal, Date, Datetime, Decimal, Duration, Encoder, F16, File, Geometry, Interval, Number,
    Object, Point, Polygon, Range, RecordId, RecordIdKey, RecordIdKeyRange, SharedKeys, Time,
    Timestamp, Uuid, Value, Vector, encode,
};

fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Object(fields.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())
}

/// A value of every kind, and of every form a kind is written in.
fn values() -> Vec<Value> {
    let number = Value::Number;
    let point = |x: f64, y: f64| Point { x, y };
    let ring = vec![point(0.0, 0.0), point(4.0, 0.0), point(4.0, 4.0), point(0.0, 0.0)];
    let polygon = Polygon { exterior: ring.clone(), interiors: vec![ring.clone()] };
    let record = |key| Value::RecordId(RecordId { table: "dog".to_owned(), key });
    let keys = Box::new(RecordIdKeyRange {
        start: Bound::Included(RecordIdKey::Number(1)),
        end: Bound::Excluded(RecordIdKey::String("z".to_owned())),
    });
    let range = Range { start: Bound::Included(1.into()), end: Bound::Unbounded };
    vec![
        Value::None,
        Value::Null,
        Value::Bool(true),
        Value::from(7),
        Value::from(-2049),
        Value::from(i64::MIN),
        Value::from(0.1),
        number(Number::F32(1.5)),
        number(Number::F16(F16::from_f32(0.5))),
        number(Number::I8(-8)),
        number(Number::U8(8)),
        number(Number::I16(-16)),
        number(Number::U16(16)),
        number(Number::I32(-32)),
        number(Number::U32(32)),
        number(Number::U64(u64::MAX)),
        number(Number::Decimal(Decimal::new(12345, 2, true).unwrap())),
        number(Number::BigDecimal(BigDecimal::new(&[0x39, 0x30, 0x7f], 40).unwrap())),
        Value::from("a"),
        Value::from("a string long enough to be pointed at"),
        Value::Bytes(vec![1, 2, 3]),
        Value::Duration(Duration { secs: 90, nanos: 5 }),
        Value::Datetime(Datetime { secs: 1_700_000_000, nanos: 5 }),
        Value::Uuid(Uuid([7; 16])),
        Value::Date(Date { days: 20_000 }),
        Value::Time(Time { nanos: 3_600_000_000_000 }),
        Value::Timestamp(Timestamp { secs: -1, nanos: 999_999_999 }),
        Value::Interval(Interval { months: 1, days: -2, micros: 3 }),
        Value::Table("dog".to_owned()),
        record(RecordIdKey::Number(1)),
        record(RecordIdKey::Uuid(Uuid([9; 16]))),
        record(RecordIdKey::Array(vec![1.into(), "a".into()])),
        record(RecordIdKey::Object(Object::from([("a".to_owned(), 1.into())]))),
        record(RecordIdKey::Range(keys)),
        Value::File(File { bucket: "bucket".to_owned(), key: "/a/b".to_owned() }),
        Value::Range(Box::new(range)),
        Value::Regex("^a+$".to_owned()),
        Value::Set(vec![1.into(), 2.into(), "a".into()]),
        Value::Geometry(Geometry::Point(point(1.0, 2.0))),
        Value::Geometry(Geometry::Line(ring.clone())),
        Value::Geometry(Geometry::Polygon(polygon.clone())),
        Value::Geometry(Geometry::MultiPoint(ring.clone())),
        Value::Geometry(Geometry::MultiLine(vec![ring.clone(), ring])),
        Value::Geometry(Geometry::MultiPolygon(vec![polygon.clone()])),
        Value::Geometry(Geometry::Collection(vec![Geometry::Polygon(polygon)])),
        Value::Vector(Vector::F32(vec![0.25, -0.5, 1.0])),
        Value::Vector(Vector::F64(vec![0.25, -0.5])),
        Value::Vector(Vector::quantized_f16(&[0.25, -0.5, 1.0])),
        Value::Vector(Vector::quantized_i8(&[0.25, -0.5, 1.0])),
        Value::Vector(Vector::quantized_bits(&[0.25, -0.5, 1.0, 0.1, -0.1, 0.2, 0.3, -1.0, 1.0])),
        Value::Array(vec![1.into(), "abc".into(), Value::Array(vec![Value::Null])]),
        Value::Array((0..3000).map(Value::from).collect()),
        object([("name", "Milou".into()), ("visits", 41.into())]),
        object([
            ("name", "Milou".into()),
            ("visits", 42.into()),
            ("inner", object([("name", "Milou".into()), ("tags", Value::Array(vec![]))])),
        ]),
    ]
}

/// Two parts as the targets cut their input: the length of the first, then both.
fn joined(first: &[u8], second: &[u8]) -> Vec<u8> {
    let length = u16::try_from(first.len()).expect("a seed is short");
    [&length.to_le_bytes()[..], first, second].concat()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = std::env::args().nth(1).unwrap_or_else(|| "fuzz/corpus".to_owned());
    let write = |target: &str, i: usize, data: &[u8]| {
        let dir = Path::new(&root).join(target);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join(format!("seed-{i:02}")), data)
    };

    let values = values();
    let documents = values.iter().map(encode).collect::<Result<Vec<_>, _>>()?;
    let paths = ["name", "inner.name", "inner.tags[0]", "[1]", "[2][0]", "missing.deeper"];
    for (i, (value, document)) in values.iter().zip(&documents).enumerate() {
        write("read", i, document)?;
        write("json", i, to_json(value).to_string().as_bytes())?;
        // Every document with the one after it, and the last two the other way round.
        let next = &documents[(i + 1) % documents.len()];
        // The largest document is over what two bytes of length take.
        if let (Ok(_), Ok(_)) = (u16::try_from(document.len()), u16::try_from(next.len())) {
            write("delta", i, &joined(document, next))?;
            let path = paths[i % paths.len()].as_bytes();
            write("update", i, &joined(next, &joined(path, document)))?;
        }

        let mut keys = SharedKeys::new();
        let mut encoder = Encoder::new().with_shared_keys(&mut keys);
        encoder.write_value(value)?;
        let with_keys = encoder.finish()?;
        write("shared", i, &joined(&keys.to_state()?, &with_keys))?;
    }
    println!("{} seeds for each target in {root}", values.len());
    Ok(())
}
