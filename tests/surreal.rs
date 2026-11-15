//! The keys of values against SurrealDB's own types (`surrealdb-types`, a dev-dependency):
//! its ordered encoding of numbers, and the order it gives its values, for every type.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::ops::Bound;

use fleece::key::encode_key;
use fleece::{
    BigDecimal, Date, Datetime, Decimal, Duration, F16, File, Geometry, Interval, Number, Point,
    Polygon, Range, RecordId, RecordIdKey, RecordIdKeyRange, Time, Timestamp, Uuid, Value,
};
use proptest::collection::{btree_map, vec};
use proptest::prelude::*;
use surrealdb_types as surreal;

fn to_surreal_array(items: &[Value]) -> surreal::Array {
    surreal::Array::from(items.iter().map(to_surreal).collect::<Vec<surreal::Value>>())
}

fn to_surreal_object(fields: &fleece::Object) -> surreal::Object {
    let fields: BTreeMap<String, surreal::Value> =
        fields.iter().map(|(key, value)| (key.clone(), to_surreal(value))).collect();
    surreal::Object::from(fields)
}

fn to_surreal_uuid(uuid: &Uuid) -> surreal::Uuid {
    surreal::Uuid::try_from(uuid.to_string()).unwrap()
}

fn to_surreal_bound<T, S>(bound: &Bound<T>, convert: impl Fn(&T) -> S) -> Bound<S> {
    match bound {
        Bound::Unbounded => Bound::Unbounded,
        Bound::Included(value) => Bound::Included(convert(value)),
        Bound::Excluded(value) => Bound::Excluded(convert(value)),
    }
}

fn to_surreal_key(key: &RecordIdKey) -> surreal::RecordIdKey {
    match key {
        RecordIdKey::Number(v) => surreal::RecordIdKey::Number(*v),
        RecordIdKey::String(v) => surreal::RecordIdKey::String(v.clone()),
        RecordIdKey::Uuid(v) => surreal::RecordIdKey::Uuid(to_surreal_uuid(v)),
        RecordIdKey::Array(v) => surreal::RecordIdKey::Array(to_surreal_array(v)),
        RecordIdKey::Object(v) => surreal::RecordIdKey::Object(to_surreal_object(v)),
        RecordIdKey::Range(range) => {
            surreal::RecordIdKey::Range(Box::new(surreal::RecordIdKeyRange {
                start: to_surreal_bound(&range.start, to_surreal_key),
                end: to_surreal_bound(&range.end, to_surreal_key),
            }))
        }
    }
}

fn line(points: &[Point]) -> geo_types::LineString<f64> {
    geo_types::LineString::from(points.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>())
}

fn polygon(polygon: &Polygon) -> geo_types::Polygon<f64> {
    geo_types::Polygon::new(
        line(&polygon.exterior),
        polygon.interiors.iter().map(|r| line(r)).collect(),
    )
}

fn to_surreal_geometry(geometry: &Geometry) -> surreal::Geometry {
    let point = |p: &Point| geo_types::Point::new(p.x, p.y);
    match geometry {
        Geometry::Point(v) => surreal::Geometry::Point(point(v)),
        Geometry::Line(v) => surreal::Geometry::Line(line(v)),
        Geometry::Polygon(v) => surreal::Geometry::Polygon(polygon(v)),
        Geometry::MultiPoint(v) => {
            surreal::Geometry::MultiPoint(geo_types::MultiPoint::new(v.iter().map(point).collect()))
        }
        Geometry::MultiLine(v) => surreal::Geometry::MultiLine(geo_types::MultiLineString::new(
            v.iter().map(|l| line(l)).collect(),
        )),
        Geometry::MultiPolygon(v) => surreal::Geometry::MultiPolygon(geo_types::MultiPolygon::new(
            v.iter().map(polygon).collect(),
        )),
        Geometry::Collection(v) => {
            surreal::Geometry::Collection(v.iter().map(to_surreal_geometry).collect())
        }
    }
}

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
        Value::Uuid(v) => surreal::Value::Uuid(to_surreal_uuid(v)),
        Value::Table(v) => surreal::Value::Table(surreal::Table::new(v.clone())),
        Value::Date(v) => surreal::Value::Date(surreal::Date::from_days(v.days).unwrap()),
        Value::Time(v) => surreal::Value::Time(surreal::Time::from_nanos(v.nanos).unwrap()),
        Value::Timestamp(v) => {
            surreal::Value::Timestamp(surreal::Timestamp::from_parts(v.secs, v.nanos).unwrap())
        }
        Value::Interval(v) => {
            surreal::Value::Interval(surreal::Interval::new(v.months, v.days, v.micros))
        }
        Value::Array(items) => surreal::Value::Array(to_surreal_array(items)),
        Value::Set(items) => {
            let items: Vec<surreal::Value> = items.iter().map(to_surreal).collect();
            surreal::Value::Set(surreal::Set::from(items))
        }
        Value::Object(fields) => surreal::Value::Object(to_surreal_object(fields)),
        Value::RecordId(v) => surreal::Value::RecordId(surreal::RecordId {
            table: surreal::Table::new(v.table.clone()),
            key: to_surreal_key(&v.key),
        }),
        Value::File(v) => surreal::Value::File(surreal::File::new(v.bucket.clone(), v.key.clone())),
        Value::Range(v) => surreal::Value::Range(Box::new(surreal::Range {
            start: to_surreal_bound(&v.start, to_surreal),
            end: to_surreal_bound(&v.end, to_surreal),
        })),
        Value::Regex(v) => surreal::Value::Regex(v.parse().unwrap()),
        Value::Geometry(v) => surreal::Value::Geometry(to_surreal_geometry(v)),
        Value::Vector(_) => panic!("SurrealDB has no vector"),
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

/// A few coordinates, the two zeros among them, so that geometries share many.
fn points(most: usize) -> impl Strategy<Value = Vec<Point>> {
    let coordinate = prop_oneof![Just(-1.5), Just(-0.0), Just(0.0), Just(1.0), Just(2.5)];
    vec((coordinate.clone(), coordinate).prop_map(|(x, y)| Point { x, y }), 0..most)
}

/// A ring that ends where it starts, as `geo-types` makes every ring of a polygon.
fn ring() -> impl Strategy<Value = Vec<Point>> {
    points(3).prop_map(|mut points| {
        points.extend(points.first().copied());
        points
    })
}

fn geometry() -> impl Strategy<Value = Geometry> {
    let polygon = || {
        (ring(), vec(ring(), 0..3))
            .prop_map(|(exterior, interiors)| Polygon { exterior, interiors })
    };
    let shape = prop_oneof![
        points(2).prop_map(|points| Geometry::Point(points.first().copied().unwrap_or_default())),
        points(4).prop_map(Geometry::Line),
        polygon().prop_map(Geometry::Polygon),
        points(4).prop_map(Geometry::MultiPoint),
        vec(points(3), 0..3).prop_map(Geometry::MultiLine),
        vec(polygon(), 0..3).prop_map(Geometry::MultiPolygon),
    ];
    shape.prop_recursive(2, 8, 3, |inner| vec(inner, 0..3).prop_map(Geometry::Collection))
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
        (-3i64..3).prop_map(RecordIdKey::Number),
        "[a-c]{0,2}".prop_map(RecordIdKey::String),
        (0..3u8).prop_map(|byte| RecordIdKey::Uuid(Uuid([byte; 16]))),
    ];
    prop_oneof![
        simple.clone(),
        vec(inner.clone(), 0..3).prop_map(RecordIdKey::Array),
        btree_map("[a-c]{0,2}", inner, 0..3).prop_map(RecordIdKey::Object),
        (bound(simple.clone()), bound(simple)).prop_map(|(start, end)| {
            RecordIdKey::Range(Box::new(RecordIdKeyRange { start, end }))
        }),
    ]
}

/// Values of every type SurrealDB has, from few enough values that two of them are often
/// close: equal, or one the start of the other.
fn value() -> impl Strategy<Value = Value> {
    // Within what `chrono` holds, which SurrealDB's dates and times are made of.
    let secs = -8_000_000_000_000i64..8_000_000_000_000;
    let nanos = 0..1_000_000_000u32;
    // Lengths that months, days and microseconds make in more than one way.
    let day = 86_400_000_000i64;
    let interval =
        (-2..3i32, -61..62i32, -31..32i64).prop_map(move |(months, days, days_of_micros)| {
            Interval { months, days, micros: days_of_micros * day }
        });
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
        // Tables and the tables of records are named apart: see the test of a table and
        // its own records below.
        "[a-c]{1,3}".prop_map(Value::Table),
        (-90_000_000i32..90_000_000).prop_map(|days| Value::Date(Date { days })),
        (0..=Time::MAX_NANOS).prop_map(|nanos| Value::Time(Time { nanos })),
        (secs, nanos).prop_map(|(secs, nanos)| Value::Timestamp(Timestamp { secs, nanos })),
        interval.prop_map(Value::Interval),
        ("[a-c]{0,2}", "/[a-c]{0,2}").prop_map(|(bucket, key)| Value::File(File { bucket, key })),
        "[a-c]{1,3}".prop_map(Value::Regex),
        geometry().prop_map(Value::Geometry),
    ];
    leaf.prop_recursive(3, 24, 4, |inner| {
        prop_oneof![
            vec(inner.clone(), 0..4).prop_map(Value::Array),
            vec(inner.clone(), 0..4).prop_map(Value::Set),
            btree_map("[a-c\\x00]{0,2}", inner.clone(), 0..4).prop_map(Value::Object),
            (bound(inner.clone()), bound(inner.clone()))
                .prop_map(|(start, end)| Value::Range(Box::new(Range { start, end }))),
            ("[x-z]{1,2}", record_key(inner))
                .prop_map(|(table, key)| Value::RecordId(RecordId { table, key })),
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

    #[test]
    fn a_table_sorts_with_the_records_of_tables(
        table in "[a-c]{1,2}",
        of in "[a-c]{1,2}",
        key in -3i64..3,
    ) {
        let record = RecordId { table: of.clone(), key: RecordIdKey::Number(key) };
        let (table, record) = (Value::Table(table), Value::RecordId(record));
        let ours = encode_key(&table).cmp(&encode_key(&record));
        // SurrealDB calls a table equal to every record of it. Here it comes just before
        // them, and every other comparison is SurrealDB's.
        match to_surreal(&table).cmp(&to_surreal(&record)) {
            Ordering::Equal => prop_assert_eq!(ours, Ordering::Less),
            expected => prop_assert_eq!(ours, expected),
        }
    }
}
