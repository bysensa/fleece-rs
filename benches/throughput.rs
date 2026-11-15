//! Times and sizes of the main operations on one realistic document, next to `serde_json`
//! on the JSON of the same data. Run with `cargo bench`.

use std::hint::black_box;
use std::time::Instant;

use fleece::delta::{append_delta, diff};
use fleece::{
    Datetime, Doc, Geometry, KeyPath, Number, Object, Point, RecordId, RecordIdKey, Value, compact,
    decode, encode,
};

/// Runs `operation` for a third of a second and prints the time of one run, and the
/// megabytes of `bytes` it gets through in a second.
fn measure<T>(name: &str, bytes: usize, mut operation: impl FnMut() -> T) {
    let start = Instant::now();
    let mut runs = 0u32;
    while start.elapsed().as_millis() < 300 {
        black_box(operation());
        runs += 1;
    }
    let seconds = start.elapsed().as_secs_f64() / f64::from(runs);
    let rate = bytes as f64 / seconds / 1e6;
    println!("{name:<52} {:>11.2} µs {rate:>9.0} MB/s", seconds * 1e6);
}

fn object<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Object(fields.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())
}

fn record(i: i64) -> Value {
    object([
        ("id", Value::RecordId(RecordId { table: "user".into(), key: RecordIdKey::Number(i) })),
        ("name", format!("User Number {i}").into()),
        ("email", format!("user{i}@example.com").into()),
        ("age", Value::Number(Number::U8((i % 90) as u8))),
        ("score", (i as f64 * 0.37).into()),
        ("active", (i % 3 == 0).into()),
        ("tags", vec!["alpha".into(), "beta".into(), format!("group{}", i % 10).into()].into()),
        ("created", Value::Datetime(Datetime { secs: 1_700_000_000 + i * 3600, nanos: 0 })),
        (
            "address",
            object([
                ("city", format!("City {}", i % 50).into()),
                ("zip", format!("{:05}", i * 7 % 100_000).into()),
                ("geo", Value::Geometry(Geometry::Point(Point { x: i as f64 / 10.0, y: 51.5 }))),
            ]),
        ),
    ])
}

/// Objects nested `depth` deep, each with 20 fields beside the next level.
fn nested(depth: usize, leaf: i64) -> Value {
    let mut value = Value::from(leaf);
    for _ in 0..depth {
        let mut fields: Object =
            (0..20).map(|i| (format!("field{i:02}"), Value::from("some text"))).collect();
        fields.insert("next".to_owned(), value);
        value = Value::Object(fields);
    }
    value
}

fn main() {
    let value = Value::Array((0..1000).map(record).collect());
    let bytes = encode(&value).unwrap();
    let json = fleece::json::to_json(&value);
    let text = json.to_string();
    println!("1000 records: {} bytes of Fleece, {} bytes of JSON\n", bytes.len(), text.len());

    let path: KeyPath = "[500].address.city".parse().unwrap();
    measure("fleece: encode a value", bytes.len(), || encode(&value).unwrap());
    measure("fleece: check a document", bytes.len(), || Doc::new(&bytes).unwrap());
    measure("fleece: check and decode to a value", bytes.len(), || decode(&bytes).unwrap());
    measure("fleece: read one field in place, unchecked", bytes.len(), || {
        path.eval(Doc::trusted(&bytes).root()).unwrap().as_str().unwrap().len()
    });
    measure("fleece: check, then read one field", bytes.len(), || {
        path.eval(Doc::new(&bytes).unwrap().root()).unwrap().as_str().unwrap().len()
    });
    measure("serde_json: write text", text.len(), || json.to_string());
    measure("serde_json: parse text", text.len(), || {
        serde_json::from_str::<serde_json::Value>(&text).unwrap()
    });
    measure("serde_json: parse, then read one field", text.len(), || {
        let json: serde_json::Value = serde_json::from_str(&text).unwrap();
        json[500]["address"]["city"].as_str().unwrap().len()
    });

    println!();
    let Value::Array(mut records) = value.clone() else { unreachable!() };
    let Value::Object(fields) = &mut records[500] else { unreachable!() };
    fields.insert("age".to_owned(), Value::Number(Number::U8(99)));
    let changed = Value::Array(records);
    let delta = append_delta(&bytes, &changed).unwrap();
    let appended = [&bytes[..], &delta[..]].concat();
    println!("one field of one record changed: a delta of {} bytes", delta.len());
    measure("fleece: delta to append", bytes.len(), || append_delta(&bytes, &changed).unwrap());
    measure("fleece: delta as a value", bytes.len(), || diff(&value, &changed).unwrap());
    measure("fleece: compact base and delta", appended.len(), || {
        compact(&Doc::trusted(&appended)).unwrap()
    });

    println!();
    for depth in [25, 50, 100] {
        let (old, new) = (nested(depth, 1), nested(depth, 2));
        let base = encode(&old).unwrap();
        let name = format!("fleece: delta at the bottom of {depth} levels");
        measure(&name, base.len(), || append_delta(&base, &new).unwrap());
    }
}
