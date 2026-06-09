//! Runtime helpers for user-specified decimal exact numeric type envelopes.

use rust_decimal::Decimal;

use crate::ast::DecimalType;

/// Return true when `value` can be represented exactly by `decimal_type`.
pub(crate) fn decimal_fits_type(value: Decimal, decimal_type: DecimalType) -> bool {
    decimal_fits(value, decimal_type).is_some()
}

/// Round `value` to the target scale and return it if it fits the requested
/// precision/scale envelope.
pub(crate) fn round_decimal_to_type(value: Decimal, decimal_type: DecimalType) -> Option<Decimal> {
    let rounded = value.round_dp(u32::from(decimal_type.scale));
    decimal_fits(rounded, decimal_type)
}

fn decimal_fits(value: Decimal, decimal_type: DecimalType) -> Option<Decimal> {
    let normalized = value.normalize();
    let value_scale = normalized.scale();
    let target_scale = u32::from(decimal_type.scale);
    if value_scale > target_scale {
        return None;
    }
    let factor = pow10_i128(target_scale - value_scale)?;
    let scaled = normalized.mantissa().checked_mul(factor)?;
    if decimal_digit_count(scaled) <= decimal_type.precision {
        Some(normalized)
    } else {
        None
    }
}

fn pow10_i128(exp: u32) -> Option<i128> {
    let mut value = 1_i128;
    for _ in 0..exp {
        value = value.checked_mul(10)?;
    }
    Some(value)
}

fn decimal_digit_count(value: i128) -> u16 {
    let mut magnitude = value.unsigned_abs();
    if magnitude == 0 {
        return 1;
    }
    let mut digits = 0_u16;
    while magnitude > 0 {
        digits += 1;
        magnitude /= 10;
    }
    digits
}

#[cfg(test)]
mod tests {
    use super::{decimal_fits_type, round_decimal_to_type};
    use crate::ast::DecimalType;

    fn ty(precision: u16, scale: u16) -> DecimalType {
        DecimalType::new(precision, scale).expect("valid decimal type")
    }

    #[test]
    fn decimal_fit_allows_padding_to_target_scale() {
        assert!(decimal_fits_type("1.2".parse().unwrap(), ty(3, 2)));
        assert!(decimal_fits_type("1.20".parse().unwrap(), ty(3, 2)));
    }

    #[test]
    fn decimal_fit_rejects_fractional_precision_loss() {
        assert!(!decimal_fits_type("1.234".parse().unwrap(), ty(3, 2)));
    }

    #[test]
    fn decimal_fit_rejects_integer_precision_overflow() {
        assert!(decimal_fits_type("9.99".parse().unwrap(), ty(3, 2)));
        assert!(!decimal_fits_type("10.00".parse().unwrap(), ty(3, 2)));
    }

    #[test]
    fn decimal_rounding_checks_precision_after_scale_conversion() {
        assert_eq!(
            round_decimal_to_type("1.235".parse().unwrap(), ty(3, 2))
                .expect("rounded decimal fits")
                .to_string(),
            "1.24"
        );
        assert!(round_decimal_to_type("9.995".parse().unwrap(), ty(3, 2)).is_none());
    }
}
