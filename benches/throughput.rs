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
    // `cargo bench -- --profile-encode` (or `-check`, `-decode`): nothing but that one
    // operation for 8 seconds, for a profiler to sample.
    let profiled =
        std::env::args().find_map(|arg| arg.strip_prefix("--profile-").map(String::from));
    if let Some(operation) = profiled {
        let doc = Doc::new(&bytes).unwrap();
        let start = Instant::now();
        while start.elapsed().as_secs() < 8 {
            match operation.as_str() {
                "encode" => drop(black_box(encode(black_box(&value)).unwrap())),
                "check" => black_box(Doc::trusted(black_box(&bytes)).validate()).unwrap(),
                "decode" => drop(black_box(Value::from_ref(black_box(doc).root()).unwrap())),
                _ => panic!("--profile-encode, --profile-check or --profile-decode"),
            }
        }
        return;
    }
    let json = fleece::json::to_json(&value);
    let text = json.to_string();
    println!("1000 records: {} bytes of Fleece, {} bytes of JSON\n", bytes.len(), text.len());

    let path: KeyPath = "[500].address.city".parse().unwrap();
    measure("fleece: encode a value", bytes.len(), || encode(&value).unwrap());
    let one = record(500);
    let one_len = encode(&one).unwrap().len();
    measure("fleece: encode one record", one_len, || encode(&one).unwrap());
    let one_json = fleece::json::to_json(&one);
    measure("serde_json: write one record", one_json.to_string().len(), || one_json.to_string());
    measure("fleece: check a document", bytes.len(), || Doc::new(&bytes).unwrap());
    measure("fleece: check and decode to a value", bytes.len(), || decode(&bytes).unwrap());
    let checked = Doc::new(&bytes).unwrap();
    measure("fleece: decode a checked document", bytes.len(), || {
        Value::from_ref(checked.root()).unwrap()
    });
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

    // A change as a store of whole values needs it: the new document, compact.
    let (path, age): (KeyPath, _) = ("[500].age".parse().unwrap(), Value::Number(Number::U8(99)));
    let mut patch = fleece::Patch::new();
    patch.set(&path, &age);
    measure("fleece: update one field without a decode", bytes.len(), || {
        fleece::update(&Doc::trusted(&bytes), &patch).unwrap()
    });
    measure("fleece: decode, change, encode", bytes.len(), || {
        let Value::Array(mut records) = Value::from_ref(Doc::trusted(&bytes).root()).unwrap()
        else {
            unreachable!()
        };
        let Value::Object(fields) = &mut records[500] else { unreachable!() };
        fields.insert("age".to_owned(), age.clone());
        encode(&Value::Array(records)).unwrap()
    });
    let one_bytes = encode(&one).unwrap();
    let (one_path, _): (KeyPath, ()) = ("age".parse().unwrap(), ());
    let mut one_patch = fleece::Patch::new();
    one_patch.set(&one_path, &age);
    measure("fleece: update one field of one record", one_bytes.len(), || {
        fleece::update(&Doc::trusted(&one_bytes), &one_patch).unwrap()
    });
    measure("fleece: key of an index entry (a string)", 20, || {
        fleece::key::encode_key(&Value::from("user500@example.com"))
    });
    measure("fleece: key of an index entry (a float)", 8, || {
        fleece::key::encode_key(&Value::from(185.37))
    });

    let mut shared = fleece::SharedKeys::new();
    let mut encoder = fleece::Encoder::new().with_shared_keys(&mut shared);
    encoder.write_value(&one).unwrap();
    let with_shared = encoder.finish().unwrap().len();
    println!("\none record: {one_len} bytes, {with_shared} with shared keys");

    // An embedding in a document: what it takes, and what a distance to a query costs
    // where it lies, against decoding it first.
    println!();
    let embedding: Vec<f32> = (0..768).map(|i| ((i * 37 % 101) as f32 - 50.0) / 400.0).collect();
    let query: Vec<f32> = (0..768).map(|i| ((i * 53 % 97) as f32 - 48.0) / 400.0).collect();
    let note = |vector: fleece::Vector| {
        let fields = [
            ("title".to_owned(), Value::from("Tintin in Tibet")),
            ("embedding".to_owned(), Value::Vector(vector)),
        ];
        encode(&Value::Object(Object::from(fields))).unwrap()
    };
    let array = Value::Array(embedding.iter().map(|v| Value::Number(Number::F32(*v))).collect());
    println!("768 dimensions as an array of f32: {} bytes", encode(&array).unwrap().len());
    let vector_path: KeyPath = "embedding".parse().unwrap();
    for (name, vector) in [
        ("f32", fleece::Vector::F32(embedding.clone())),
        ("f16", fleece::Vector::quantized_f16(&embedding)),
        ("i8", fleece::Vector::quantized_i8(&embedding)),
        ("bits", fleece::Vector::quantized_bits(&embedding)),
    ] {
        let stored = note(vector);
        let label = format!("vector {name}, {} bytes: dot in place", stored.len());
        measure(&label, stored.len(), || {
            let field = vector_path.eval(Doc::trusted(&stored).root()).unwrap();
            field.as_vector().unwrap().dot(&query).unwrap()
        });
    }
    let stored = note(fleece::Vector::F32(embedding.clone()));
    measure("vector f32: cosine in place", stored.len(), || {
        let field = vector_path.eval(Doc::trusted(&stored).root()).unwrap();
        field.as_vector().unwrap().cosine(&query).unwrap()
    });
    measure("vector f32: decode the document, then dot", stored.len(), || {
        let Value::Object(fields) = Value::from_ref(Doc::trusted(&stored).root()).unwrap() else {
            unreachable!()
        };
        let Some(Value::Vector(fleece::Vector::F32(values))) = fields.get("embedding") else {
            unreachable!()
        };
        values.iter().zip(&query).map(|(a, b)| a * b).sum::<f32>()
    });
    measure("plain slices: dot of 768 f32, one sum", 768 * 4, || {
        black_box(&embedding).iter().zip(black_box(&query)).map(|(a, b)| a * b).sum::<f32>()
    });

    println!();
    for depth in [25, 50, 100] {
        let (old, new) = (nested(depth, 1), nested(depth, 2));
        let base = encode(&old).unwrap();
        let name = format!("fleece: delta at the bottom of {depth} levels");
        measure(&name, base.len(), || append_delta(&base, &new).unwrap());
    }
}
