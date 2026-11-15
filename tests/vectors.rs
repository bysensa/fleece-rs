//! Vectors of embeddings: what is stored, what a distance in place gives, and what the
//! smaller element types cost in precision.

use fleece::{Doc, Element, F16, Object, Value, Vector, decode, encode};
use proptest::collection::vec;
use proptest::prelude::*;

/// A unit vector of `dims` dimensions, the same for the same `seed`.
fn unit(dims: usize, seed: u64) -> Vec<f32> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    };
    let values: Vec<f32> = (0..dims).map(|_| next()).collect();
    let norm = values.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
    values.iter().map(|v| (f64::from(*v) / norm) as f32).collect()
}

/// The dot product, the squared distance and the cosine of two vectors, in doubles and
/// one dimension after another: what the kernels are held against.
fn reference(a: &[f32], b: &[f32]) -> (f64, f64, f64) {
    let pairs = || a.iter().zip(b).map(|(a, b)| (f64::from(*a), f64::from(*b)));
    let dot: f64 = pairs().map(|(a, b)| a * b).sum();
    let distance: f64 = pairs().map(|(a, b)| (a - b) * (a - b)).sum();
    let own: f64 = pairs().map(|(a, _)| a * a).sum();
    let other: f64 = pairs().map(|(_, b)| b * b).sum();
    let lengths = (own * other).sqrt();
    (dot, distance, if lengths > 0.0 { dot / lengths } else { 0.0 })
}

fn close(found: f32, expected: f64, within: f64) {
    assert!((f64::from(found) - expected).abs() <= within, "{found} is not {expected}");
}

/// The document of one vector, and what `read` makes of the vector in it.
fn in_place<R>(vector: Vector, read: impl FnOnce(fleece::VectorRef<'_>) -> R) -> R {
    let bytes = encode(&Value::Vector(vector)).unwrap();
    read(Doc::new(&bytes).unwrap().root().as_vector().unwrap())
}

#[test]
fn a_vector_takes_its_elements_and_six_bytes_more() {
    let values = unit(768, 1);
    let size = |vector: Vector| encode(&Value::Vector(vector)).unwrap().len();
    // The header of the kind, of the data and the type of the elements, and the pointer
    // that ends the document.
    assert_eq!(size(Vector::F32(values.clone())), 768 * 4 + 8);
    assert_eq!(size(Vector::quantized_f16(&values)), 768 * 2 + 8);
    assert_eq!(size(Vector::quantized_i8(&values)), 768 + 8);
    assert_eq!(size(Vector::quantized_bits(&values)), 768 / 8 + 8);
    // The same numbers as an array: a slot and a float behind it for each.
    let array = Value::Array(values.iter().map(|v| Value::from(f64::from(*v))).collect());
    assert!(encode(&array).unwrap().len() > 768 * 8);
}

#[test]
fn distances_in_place_are_the_distances() {
    for dims in [1, 7, 8, 9, 128, 768, 771] {
        let (stored, query) = (unit(dims, 3), unit(dims, 4));
        let (dot, distance, cosine) = reference(&stored, &query);
        in_place(Vector::F32(stored.clone()), |vector| {
            assert_eq!((vector.len(), vector.element()), (dims, Element::F32));
            close(vector.dot(&query).unwrap(), dot, 1e-5);
            close(vector.l2_squared(&query).unwrap(), distance, 1e-5);
            close(vector.cosine(&query).unwrap(), cosine, 1e-5);
            assert_eq!(vector.to_f32(), stored);
            assert_eq!(vector.get(dims - 1), stored.last().copied());
            assert_eq!(vector.get(dims), None);
            // A query of another size is not compared.
            assert_eq!(vector.dot(&query[1..]), None);
        });
        let doubles = Vector::F64(stored.iter().map(|v| f64::from(*v)).collect());
        in_place(doubles, |vector| close(vector.dot(&query).unwrap(), dot, 1e-5));
    }
}

#[test]
fn smaller_elements_keep_the_ranking_of_unit_vectors() {
    let (stored, query) = (unit(768, 5), unit(768, 6));
    let (dot, distance, cosine) = reference(&stored, &query);
    // Half the size: the third decimal digit of an element is the last one kept.
    in_place(Vector::quantized_f16(&stored), |vector| {
        close(vector.dot(&query).unwrap(), dot, 1e-4);
        close(vector.l2_squared(&query).unwrap(), distance, 1e-4);
        close(vector.cosine(&query).unwrap(), cosine, 1e-4);
        let norm = vector.to_f32().iter().map(|v| f64::from(*v).powi(2)).sum::<f64>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "{norm}");
    });
    // A quarter of the size: a step of 1/127 an element.
    in_place(Vector::quantized_i8(&stored), |vector| {
        close(vector.dot(&query).unwrap(), dot, 5e-3);
        close(vector.cosine(&query).unwrap(), cosine, 5e-3);
    });
    // A bit a dimension tells near from far, and no more: a vector agrees with itself in
    // every sign and with another unit vector in about half of them.
    let (bits, other) = (Vector::quantized_bits(&stored), Vector::quantized_bits(&query));
    let (bits_bytes, other_bytes) =
        (encode(&Value::Vector(bits)).unwrap(), encode(&Value::Vector(other)).unwrap());
    let bits = Doc::new(&bits_bytes).unwrap().root().as_vector().unwrap();
    let other = Doc::new(&other_bytes).unwrap().root().as_vector().unwrap();
    assert_eq!(bits.hamming(&bits), Some(0));
    let apart = bits.hamming(&other).unwrap();
    assert!((300..470).contains(&apart), "{apart}");
    assert!(bits.dot(&stored).unwrap() > bits.dot(&query).unwrap());
    in_place(Vector::F32(stored), |vector| assert_eq!(vector.hamming(&bits), None));
}

#[test]
fn a_prefix_is_the_shorter_embedding() {
    let (stored, query) = (unit(768, 7), unit(768, 8));
    for dims in [128, 256, 512, 768] {
        let (dot, _, cosine) = reference(&stored[..dims], &query[..dims]);
        in_place(Vector::F32(stored.clone()), |vector| {
            let prefix = vector.prefix(dims).unwrap();
            assert_eq!(prefix.len(), dims);
            close(prefix.dot(&query[..dims]).unwrap(), dot, 1e-5);
            // A part of a unit vector is not a unit vector: the cosine divides by the
            // lengths of the parts.
            close(prefix.cosine(&query[..dims]).unwrap(), cosine, 1e-5);
            assert_eq!(prefix.to_f32(), &stored[..dims]);
        });
    }
    in_place(Vector::F32(stored.clone()), |vector| assert!(vector.prefix(769).is_none()));
    // Bits of a number of dimensions that does not fill the last byte.
    let (bits, other) = (Vector::quantized_bits(&stored), Vector::quantized_bits(&query));
    let (bits_bytes, other_bytes) =
        (encode(&Value::Vector(bits)).unwrap(), encode(&Value::Vector(other)).unwrap());
    let bits = Doc::new(&bits_bytes).unwrap().root().as_vector().unwrap();
    let other = Doc::new(&other_bytes).unwrap().root().as_vector().unwrap();
    let differ =
        |dims: usize| (0..dims).filter(|i| (stored[*i] > 0.0) != (query[*i] > 0.0)).count() as u32;
    for dims in [1, 7, 8, 13, 100, 768] {
        let (a, b) = (bits.prefix(dims).unwrap(), other.prefix(dims).unwrap());
        assert_eq!(a.hamming(&b), Some(differ(dims)), "{dims}");
        assert_eq!(a.to_vector(), Vector::quantized_bits(&stored[..dims]));
    }
}

#[test]
fn a_vector_lies_in_a_document_beside_other_fields() {
    let embedding = unit(256, 9);
    let fields = [
        ("title".to_owned(), Value::from("Tintin in Tibet")),
        ("embedding".to_owned(), Value::Vector(Vector::quantized_f16(&embedding))),
    ];
    let bytes = encode(&Value::Object(Object::from(fields.clone()))).unwrap();
    let doc = Doc::new(&bytes).unwrap();
    let field = doc.root().as_dict().unwrap().get("embedding").unwrap();
    close(field.as_vector().unwrap().dot(&embedding).unwrap(), 1.0, 1e-3);
    assert!(doc.root().as_dict().unwrap().get("title").unwrap().as_vector().is_none());
    assert_eq!(decode(&bytes).unwrap(), Value::Object(Object::from(fields)));
}

#[test]
fn f16_is_the_f16_of_half_to_and_from_f32() {
    for bits in 0..=u16::MAX {
        let (ours, theirs) = (F16(bits).to_f32(), half::f16::from_bits(bits).to_f32());
        assert_eq!(ours.to_bits(), theirs.to_bits(), "{bits:#06x}");
        if !ours.is_nan() {
            assert_eq!(F16::from_f32(ours), F16(bits), "{bits:#06x}");
        }
    }
    // The values between two f16, their halfway points and the ends of the range.
    for bits in (0..=u32::MAX).step_by(4099) {
        let value = f32::from_bits(bits);
        assert_eq!(F16::from_f32(value).0, half::f16::from_f32(value).to_bits(), "{bits:#010x}");
    }
    for value in [65_504.0, 65_519.99, 65_520.0, f32::MAX, 1e-8, 2.98e-8, f32::MIN_POSITIVE] {
        for value in [value, -value] {
            assert_eq!(F16::from_f32(value).0, half::f16::from_f32(value).to_bits(), "{value}");
        }
    }
}

proptest! {
    #[test]
    fn a_vector_reads_back_and_measures_as_its_values(
        values in vec(-1.0f32..1.0, 0..40),
        query in vec(-1.0f32..1.0, 40),
        kind in 0..5u8,
    ) {
        let vector = match kind {
            0 => Vector::F32(values.clone()),
            1 => Vector::F64(values.iter().map(|v| f64::from(*v)).collect()),
            2 => Vector::quantized_f16(&values),
            3 => Vector::quantized_i8(&values),
            _ => Vector::quantized_bits(&values),
        };
        let bytes = encode(&Value::Vector(vector.clone())).unwrap();
        prop_assert_eq!(decode(&bytes).unwrap(), Value::Vector(vector.clone()));
        let stored = Doc::new(&bytes).unwrap().root().as_vector().unwrap();
        prop_assert_eq!(stored.len(), values.len());
        prop_assert_eq!(stored.to_vector(), vector.clone());
        // Whatever the type of the elements, a distance is that of the values they stand
        // for.
        let query = &query[..values.len()];
        let (dot, distance, cosine) = reference(&vector.to_f32(), query);
        prop_assert!((f64::from(stored.dot(query).unwrap()) - dot).abs() < 1e-4);
        prop_assert!((f64::from(stored.l2_squared(query).unwrap()) - distance).abs() < 1e-4);
        prop_assert!((f64::from(stored.cosine(query).unwrap()) - cosine).abs() < 1e-4);
    }
}
