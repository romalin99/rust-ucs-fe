/// String and type conversion utilities.
///
/// Full port of Go's `pkg/conv/string.go`.
///
/// Note: Go's `pkg/conv/db_field.go` (sql.Null* helpers) is not ported
/// because Rust uses `Option<T>` instead of `database/sql.Null*` types.

/// Parse a string to `i64`, returning `default` on failure.
///
/// Mirrors Go's `StringToIntDefault`:
/// ```go
/// func StringToIntDefault(s string, def int) int
/// ```
///
/// Trims whitespace before parsing, returns `default` for empty strings
/// or strings that cannot be parsed.
///
/// # Examples
///
/// ```
/// use ucs_fe::pkg::conv::string_to_i64_default;
/// assert_eq!(string_to_i64_default("42", 0),  42);
/// assert_eq!(string_to_i64_default("",   0),  0);
/// assert_eq!(string_to_i64_default("x",  -1), -1);
/// assert_eq!(string_to_i64_default(" 7 ", 0), 7);
/// ```
pub fn string_to_i64_default(s: &str, default: i64) -> i64 {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return default;
    }
    trimmed.parse::<i64>().unwrap_or(default)
}

/// Parse a string to `i32`, returning `default` on failure.
///
/// Convenience wrapper over [`string_to_i64_default`] for the common
/// `int` (32-bit) use case that matches Go's `int`.
pub fn string_to_i32_default(s: &str, default: i32) -> i32 {
    string_to_i64_default(s, default as i64) as i32
}

/// Parse a string to `f64`, returning `default` on failure.
///
/// Mirrors the pattern used in `finance_history.go` where decimal strings
/// from USS / MCS responses are converted to numeric amounts.
pub fn string_to_f64_default(s: &str, default: f64) -> f64 {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return default;
    }
    trimmed.parse::<f64>().unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_to_i64_default() {
        assert_eq!(string_to_i64_default("42",  0),  42);
        assert_eq!(string_to_i64_default("",    0),  0);
        assert_eq!(string_to_i64_default("x",  -1), -1);
        assert_eq!(string_to_i64_default(" 7 ", 0),  7);
        assert_eq!(string_to_i64_default("-5",  0), -5);
    }

    #[test]
    fn test_string_to_f64_default() {
        assert!((string_to_f64_default("1.23", 0.0) - 1.23).abs() < 1e-9);
        assert_eq!(string_to_f64_default("", 0.5), 0.5);
        assert_eq!(string_to_f64_default("nan", 99.0), 99.0);
    }
}
