/// String and type conversion utilities.
///
/// Full port of Go's `pkg/conv/`:
///   - `string.go`   → string-to-numeric parse helpers
///   - `db_field.go` → `Option<T>` unwrap helpers (Rust equivalent of Go's `sql.Null*` wrappers)
///
/// In Go, database/sql uses `sql.NullString`, `sql.NullInt64`, etc.  In Rust
/// the direct equivalent is `Option<T>`.  The helpers below mirror the Go
/// null-unwrap pattern so call-sites look and read identically.
use chrono::{DateTime, NaiveDateTime, Utc};

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
    trimmed.parse::<f64>().ok().filter(|v| !v.is_nan()).unwrap_or(default)
}

// ─────────────────────────────────────────────────────────────────────────────
// db_field.go — Option<T> null-safe unwrap helpers
//
// Mirrors Go's `pkg/conv/db_field.go` which wraps `sql.Null*` types.
// In Rust we use `Option<T>` instead.
// ─────────────────────────────────────────────────────────────────────────────

/// Unwrap `Option<String>`, returning `""` when `None`.
///
/// Mirrors Go's `conv.String(sql.NullString)`.
pub fn opt_string(v: Option<String>) -> String {
    v.unwrap_or_default()
}

/// Unwrap `Option<String>` with an explicit default.
///
/// Mirrors Go's `conv.NullString(ns, defaultVal)`.
pub fn opt_string_or(v: Option<String>, default: &str) -> String {
    v.unwrap_or_else(|| default.to_owned())
}

/// Unwrap `Option<String>`, returning `""` when `None`.
///
/// Mirrors Go's `conv.NullStringToString(ns)`.
pub fn null_string_to_string(v: Option<String>) -> String {
    opt_string(v)
}

/// Wrap a non-empty string in `Some`, `None` for empty / missing values.
///
/// Mirrors Go's `conv.NewNullString(s)`.
pub fn new_null_string(s: &str) -> Option<String> {
    if s.is_empty() { None } else { Some(s.to_owned()) }
}

/// Unwrap `Option<i64>`, returning `0` when `None`.
///
/// Mirrors Go's `conv.Int64(sql.NullInt64)`.
pub fn opt_i64(v: Option<i64>) -> i64 {
    v.unwrap_or(0)
}

/// Unwrap `Option<i64>` with an explicit default.
///
/// Mirrors Go's `conv.NullInt64(ni, defaultVal)`.
pub fn opt_i64_or(v: Option<i64>, default: i64) -> i64 {
    v.unwrap_or(default)
}

/// Wrap a non-zero `i64` in `Some`, `None` for zero.
///
/// Mirrors Go's `conv.NewNullInt64(i)`.
pub fn new_null_i64(i: i64) -> Option<i64> {
    if i == 0 { None } else { Some(i) }
}

/// Unwrap `Option<i32>`, returning `0` when `None`.
///
/// Mirrors Go's `conv.Int(sql.NullInt64)` (cast to int).
pub fn opt_i32(v: Option<i32>) -> i32 {
    v.unwrap_or(0)
}

/// Unwrap `Option<i32>` with an explicit default.
///
/// Mirrors Go's `conv.NullInt32(ni, defaultVal)`.
pub fn opt_i32_or(v: Option<i32>, default: i32) -> i32 {
    v.unwrap_or(default)
}

/// Wrap a non-zero `i32` in `Some`, `None` for zero.
///
/// Mirrors Go's `conv.NewNullInt32(i)`.
pub fn new_null_i32(i: i32) -> Option<i32> {
    if i == 0 { None } else { Some(i) }
}

/// Unwrap `Option<i16>` with an explicit default.
///
/// Mirrors Go's `conv.NullInt16(ni, defaultVal)`.
pub fn opt_i16_or(v: Option<i16>, default: i16) -> i16 {
    v.unwrap_or(default)
}

/// Wrap a non-zero `i16` in `Some`, `None` for zero.
///
/// Mirrors Go's `conv.NewNullInt16(i)`.
pub fn new_null_i16(i: i16) -> Option<i16> {
    if i == 0 { None } else { Some(i) }
}

/// Unwrap `Option<f64>`, returning `0.0` when `None`.
///
/// Mirrors Go's `conv.Float64(sql.NullFloat64)`.
pub fn opt_f64(v: Option<f64>) -> f64 {
    v.unwrap_or(0.0)
}

/// Unwrap `Option<bool>`, returning `false` when `None`.
///
/// Mirrors Go's `conv.Bool(sql.NullBool)`.
pub fn opt_bool(v: Option<bool>) -> bool {
    v.unwrap_or(false)
}

/// Wrap a `bool` in `Some` (always valid, equivalent to Go's `NewNullBool`).
pub fn new_null_bool(b: bool) -> Option<bool> {
    Some(b)
}

/// Unwrap `Option<NaiveDateTime>` with an explicit default.
///
/// Mirrors Go's `conv.NullTime(nt, defaultTime)`.
pub fn opt_datetime_or(v: Option<NaiveDateTime>, default: NaiveDateTime) -> NaiveDateTime {
    v.unwrap_or(default)
}

/// Unwrap `Option<NaiveDateTime>`, returning epoch when `None`.
///
/// Mirrors Go's `conv.Time(sql.NullTime)`.
pub fn opt_datetime(v: Option<NaiveDateTime>) -> NaiveDateTime {
    v.unwrap_or_else(|| DateTime::<Utc>::UNIX_EPOCH.naive_utc())
}

/// Format `Option<NaiveDateTime>` as a string using the given layout.
/// Returns `""` when `None`.
///
/// Mirrors Go's `conv.FormatNullTime(nt, layout)`.
///
/// `layout` uses `chrono` format strings (e.g. `"%Y-%m-%d %H:%M:%S"`).
pub fn format_opt_datetime(v: Option<NaiveDateTime>, layout: &str) -> String {
    let fmt = if layout.is_empty() { "%Y-%m-%d %H:%M:%S" } else { layout };
    v.map(|dt| dt.format(fmt).to_string()).unwrap_or_default()
}

/// Format `Option<NaiveDateTime>` as `"YYYY-MM-DD HH:MM:SS"` or `""`.
///
/// Mirrors Go's `conv.FromNullTime(nt)`.
pub fn from_opt_datetime(v: Option<NaiveDateTime>) -> String {
    format_opt_datetime(v, "%Y-%m-%d %H:%M:%S")
}

/// Wrap a non-zero `NaiveDateTime` in `Some`, `None` for the zero instant.
///
/// Mirrors Go's `conv.NewNullTime(t)`.
pub fn new_null_datetime(t: NaiveDateTime) -> Option<NaiveDateTime> {
    if t == DateTime::<Utc>::UNIX_EPOCH.naive_utc() { None } else { Some(t) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_to_i64_default() {
        assert_eq!(string_to_i64_default("42", 0), 42);
        assert_eq!(string_to_i64_default("", 0), 0);
        assert_eq!(string_to_i64_default("x", -1), -1);
        assert_eq!(string_to_i64_default(" 7 ", 0), 7);
        assert_eq!(string_to_i64_default("-5", 0), -5);
    }

    #[test]
    fn test_string_to_f64_default() {
        assert!((string_to_f64_default("1.23", 0.0) - 1.23).abs() < 1e-9);
        assert_eq!(string_to_f64_default("", 0.5), 0.5);
        assert_eq!(string_to_f64_default("nan", 99.0), 99.0);
    }
}
