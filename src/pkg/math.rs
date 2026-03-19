/// Math utility functions.
///
/// Full port of Go's `pkg/math/round.go`.

/// Round a float to 2 decimal places.
///
/// Mirrors Go's `Round2`:
/// ```go
/// func Round2(val float64) float64 {
///     return math.Round(val*100) / 100
/// }
/// ```
///
/// # Examples
///
/// ```
/// use ucs_fe::pkg::math::round2;
/// assert_eq!(round2(1.005), 1.01);
/// assert_eq!(round2(1.234), 1.23);
/// ```
pub fn round2(val: f64) -> f64 {
    (val * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_round2_basic() {
        assert_eq!(round2(1.234),  1.23);
        assert_eq!(round2(1.235),  1.24);
        assert_eq!(round2(1.0),    1.0);
        assert_eq!(round2(0.0),    0.0);
        assert_eq!(round2(-1.235), -1.24);
    }
}
