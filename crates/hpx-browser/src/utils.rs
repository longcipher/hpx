//! Shared utility functions for the browser crate.

/// Escape a string for safe interpolation into a JS single-quoted literal.
///
/// Handles `\`, `'`, `` ` ``, `\n`, `\r`, and `\t` — the characters that
/// would break a single-quoted JS string or template literal.
#[must_use]
pub fn escape_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '"' => out.push_str("\\\""),
            '`' => out.push_str("\\`"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_js_string_simple() {
        assert_eq!(escape_js_string("hello"), "hello");
    }

    #[test]
    fn escape_js_string_quotes() {
        assert_eq!(escape_js_string("it's"), "it\\'s");
    }

    #[test]
    fn escape_js_string_backslash() {
        assert_eq!(escape_js_string("a\\b"), "a\\\\b");
    }

    #[test]
    fn escape_js_string_newlines() {
        assert_eq!(escape_js_string("a\nb\r"), "a\\nb\\r");
    }

    #[test]
    fn escape_js_string_backtick() {
        assert_eq!(escape_js_string("a`b"), "a\\`b");
    }

    #[test]
    fn escape_js_string_double_quote() {
        assert_eq!(escape_js_string(r#"a"b"#), r#"a\"b"#);
    }

    #[test]
    fn escape_js_string_tab() {
        assert_eq!(escape_js_string("a\tb"), "a\\tb");
    }

    #[test]
    fn escape_js_string_empty() {
        assert_eq!(escape_js_string(""), "");
    }

    #[test]
    fn escape_js_string_mixed() {
        assert_eq!(
            escape_js_string("it's a\n\"test\""),
            "it\\'s a\\n\\\"test\\\""
        );
    }
}
