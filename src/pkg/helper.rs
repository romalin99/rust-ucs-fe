//! Decimal / numeric helper utilities.
//!
//! Partial port of Go's `pkg/helper/decimal_helper.go`.
//!
//! Go's helper heavily depends on `godror.Number` (Oracle-specific) and
//! `bson.Decimal128` (MongoDB-specific) which are not used in the Rust port.
//! This module provides the general-purpose numeric helpers in idiomatic Rust.

/// Round a `f64` value to N decimal places.
///
/// General form of Go's `Round2` (`pkg/math/round.go`).
///
/// # Examples
///
/// ```
/// use ucs_fe::pkg::helper::round_to;
/// assert!((round_to(1.2346, 2) - 1.23).abs() < 1e-9);
/// assert!((round_to(1.2346, 3) - 1.235).abs() < 1e-9);
/// ```
#[allow(clippy::cast_possible_wrap)]
pub fn round_to(val: f64, decimals: u32) -> f64 {
    let factor = 10_f64.powi(decimals as i32);
    (val * factor).round() / factor
}

/// Convert a string to `f64`, preserving up to 4 decimal places.
///
/// Mirrors Go's `FloatToDecimal128` pattern where values are formatted as
/// `"%.4f"` before being stored in Oracle `NUMBER(38,4)` columns.
pub fn parse_decimal_str(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok()
}

/// Format a `f64` as a string with exactly 4 decimal places.
///
/// Used when constructing Oracle `NUMBER(38,4)` literals.
pub fn format_decimal4(val: f64) -> String {
    format!("{val:.4}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round_to() {
        assert!((round_to(1.2346, 2) - 1.23).abs() < 1e-9);
        assert!((round_to(1.2345, 3) - 1.235).abs() < 1e-9);
        assert!((round_to(0.0, 2) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_parse_decimal_str() {
        assert_eq!(parse_decimal_str("1.23"), Some(1.23));
        assert_eq!(parse_decimal_str(""), None);
        assert_eq!(parse_decimal_str("abc"), None);
    }

    #[test]
    fn test_format_decimal4() {
        assert_eq!(format_decimal4(1.23), "1.2300");
        assert_eq!(format_decimal4(0.0), "0.0000");
        assert_eq!(format_decimal4(1.23456), "1.2346");
    }
}
