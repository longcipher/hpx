use std::borrow::Cow;

use crate::css_parser::source::SourceLocation;

/// A CSS token with its source location.
#[derive(Debug, Clone, PartialEq)]
pub struct Token<'a> {
    pub kind: TokenKind<'a>,
    pub loc: SourceLocation,
}

/// All CSS token types per CSS Syntax Level 3 §4.
#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind<'a> {
    Ident(&'a str),
    Function(&'a str),
    AtKeyword(&'a str),
    Hash {
        value: &'a str,
        is_id: bool,
    },
    String(&'a str),
    BadString,
    Url(&'a str),
    BadUrl,
    Number {
        value: f64,
        int_value: Option<i64>,
        has_sign: bool,
    },
    Percentage {
        value: f64,
        int_value: Option<i64>,
    },
    Dimension {
        value: f64,
        int_value: Option<i64>,
        unit: &'a str,
    },
    Whitespace,
    Delim(char),
    Colon,
    Semicolon,
    Comma,
    OpenSquare,
    CloseSquare,
    OpenParen,
    CloseParen,
    OpenCurly,
    CloseCurly,
    Cdo,
    Cdc,
    Eof,
}

impl<'a> TokenKind<'a> {
    pub fn is_whitespace(&self) -> bool {
        matches!(self, TokenKind::Whitespace)
    }

    pub fn is_ident(&self) -> bool {
        matches!(self, TokenKind::Ident(_))
    }
}

/// Resolve CSS escape sequences in a raw string slice.
pub fn resolve_escapes(raw: &str) -> Cow<'_, str> {
    if !raw.contains('\\') {
        return Cow::Borrowed(raw);
    }

    let mut result = String::with_capacity(raw.len());
    let mut chars = raw.chars();

    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                None => {
                    result.push(ch);
                }
                Some('\n') => {}
                Some(next) if next.is_ascii_hexdigit() => {
                    let mut hex = String::with_capacity(6);
                    hex.push(next);
                    for _ in 0..5 {
                        match chars.clone().next() {
                            Some(h) if h.is_ascii_hexdigit() => {
                                hex.push(h);
                                chars.next();
                            }
                            _ => break,
                        }
                    }
                    if let Some(&ws) = chars.as_str().as_bytes().first() {
                        if ws == b' ' || ws == b'\t' || ws == b'\n' {
                            chars.next();
                        }
                    }
                    if let Ok(code) = u32::from_str_radix(&hex, 16) {
                        if let Some(c) = char::from_u32(code) {
                            if c == '\0' {
                                result.push('\u{FFFD}');
                            } else {
                                result.push(c);
                            }
                        } else {
                            result.push('\u{FFFD}');
                        }
                    } else {
                        result.push('\u{FFFD}');
                    }
                }
                Some(next) => {
                    result.push(next);
                }
            }
        } else {
            result.push(ch);
        }
    }

    Cow::Owned(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_no_escapes() {
        let result = resolve_escapes("hello");
        assert!(matches!(result, Cow::Borrowed(_)));
        assert_eq!(result, "hello");
    }

    #[test]
    fn resolve_hex_escape() {
        assert_eq!(resolve_escapes("\\26 "), "&");
        assert_eq!(resolve_escapes("\\000026 "), "&");
    }

    #[test]
    fn resolve_simple_escape() {
        assert_eq!(resolve_escapes("\\("), "(");
        assert_eq!(resolve_escapes("hello\\.world"), "hello.world");
    }

    #[test]
    fn resolve_null_escape() {
        assert_eq!(resolve_escapes("\\0 "), "\u{FFFD}");
    }

    #[test]
    fn resolve_newline_continuation() {
        assert_eq!(resolve_escapes("hel\\\nlo"), "hello");
    }
}
