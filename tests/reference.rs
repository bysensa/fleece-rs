//! The layouts of the numeric types against the crates SurrealDB holds them with.

use fleece::{BigDecimal, Decimal, F16};
use num_bigint::BigInt;
use proptest::collection::vec;
use proptest::prelude::*;

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
