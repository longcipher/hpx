/// Source position in the CSS input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceLocation {
    pub offset: usize,
    pub line: u32,
    pub column: u32,
}

impl Default for SourceLocation {
    fn default() -> Self {
        Self {
            offset: 0,
            line: 1,
            column: 1,
        }
    }
}

/// Zero-copy input reader with CR/LF normalization and source location tracking.
pub struct SourceInput<'a> {
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
    line: u32,
    column: u32,
}

impl<'a> SourceInput<'a> {
    pub fn new(input: &'a str) -> Self {
        let bytes = input.as_bytes();
        let mut pos = 0;

        // Skip BOM (U+FEFF)
        if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            pos = 3;
        }

        Self {
            input,
            bytes,
            pos,
            line: 1,
            column: 1,
        }
    }

    pub fn location(&self) -> SourceLocation {
        SourceLocation {
            offset: self.pos,
            line: self.line,
            column: self.column,
        }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn is_eof(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    pub fn current_char(&self) -> Option<char> {
        if self.pos >= self.bytes.len() {
            return None;
        }
        let ch = self.decode_char_at(self.pos);
        Some(Self::normalize(ch))
    }

    pub fn peek_char(&self, n: usize) -> Option<char> {
        let mut pos = self.pos;
        for _ in 0..n {
            if pos >= self.bytes.len() {
                return None;
            }
            let ch = self.decode_char_at(pos);
            pos += ch.len_utf8();
            if ch == '\r' && pos < self.bytes.len() && self.bytes[pos] == b'\n' {
                pos += 1;
            }
        }
        if pos >= self.bytes.len() {
            return None;
        }
        let ch = self.decode_char_at(pos);
        Some(Self::normalize(ch))
    }

    pub fn next_char(&mut self) -> Option<char> {
        if self.pos >= self.bytes.len() {
            return None;
        }

        let ch = self.decode_char_at(self.pos);
        self.pos += ch.len_utf8();

        let normalized = Self::normalize(ch);

        if ch == '\r' {
            if self.pos < self.bytes.len() && self.bytes[self.pos] == b'\n' {
                self.pos += 1;
            }
            self.line += 1;
            self.column = 1;
            return Some('\n');
        }

        if ch == '\n' || ch == '\x0C' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }

        Some(normalized)
    }

    pub fn reconsume(&mut self) {
        if self.pos > 0 {
            let mut back = self.pos - 1;
            while back > 0 && !self.input.is_char_boundary(back) {
                back -= 1;
            }
            self.pos = back;
            if self.column > 1 {
                self.column -= 1;
            }
        }
    }

    pub fn slice(&self, start: usize, end: usize) -> &'a str {
        &self.input[start..end]
    }

    pub fn slice_from(&self, start: usize) -> &'a str {
        &self.input[start..self.pos]
    }

    pub fn starts_with_ignore_case(&self, pattern: &str) -> bool {
        let remaining = &self.bytes[self.pos..];
        if remaining.len() < pattern.len() {
            return false;
        }
        remaining[..pattern.len()]
            .iter()
            .zip(pattern.bytes())
            .all(|(&a, b)| a.eq_ignore_ascii_case(&b))
    }

    fn decode_char_at(&self, pos: usize) -> char {
        self.input[pos..].chars().next().unwrap_or('\0')
    }

    fn normalize(ch: char) -> char {
        match ch {
            '\0' => '\u{FFFD}',
            '\x0C' => '\n',
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bom_is_skipped() {
        let input = "\u{FEFF}hello";
        let mut src = SourceInput::new(input);
        assert_eq!(src.next_char(), Some('h'));
    }

    #[test]
    fn cr_normalized_to_lf() {
        let input = "a\rb";
        let mut src = SourceInput::new(input);
        assert_eq!(src.next_char(), Some('a'));
        assert_eq!(src.next_char(), Some('\n'));
        assert_eq!(src.next_char(), Some('b'));
    }

    #[test]
    fn crlf_normalized_to_single_lf() {
        let input = "a\r\nb";
        let mut src = SourceInput::new(input);
        assert_eq!(src.next_char(), Some('a'));
        assert_eq!(src.next_char(), Some('\n'));
        assert_eq!(src.next_char(), Some('b'));
    }

    #[test]
    fn null_replaced_with_replacement() {
        let input = "a\0b";
        let mut src = SourceInput::new(input);
        assert_eq!(src.next_char(), Some('a'));
        assert_eq!(src.next_char(), Some('\u{FFFD}'));
        assert_eq!(src.next_char(), Some('b'));
    }

    #[test]
    fn location_tracking() {
        let input = "ab\ncd";
        let mut src = SourceInput::new(input);
        assert_eq!(
            src.location(),
            SourceLocation {
                offset: 0,
                line: 1,
                column: 1
            }
        );
        src.next_char();
        assert_eq!(
            src.location(),
            SourceLocation {
                offset: 1,
                line: 1,
                column: 2
            }
        );
        src.next_char();
        assert_eq!(
            src.location(),
            SourceLocation {
                offset: 2,
                line: 1,
                column: 3
            }
        );
        src.next_char(); // \n
        assert_eq!(
            src.location(),
            SourceLocation {
                offset: 3,
                line: 2,
                column: 1
            }
        );
    }

    #[test]
    fn peek_ahead() {
        let input = "abc";
        let src = SourceInput::new(input);
        assert_eq!(src.current_char(), Some('a'));
        assert_eq!(src.peek_char(0), Some('a'));
        assert_eq!(src.peek_char(1), Some('b'));
        assert_eq!(src.peek_char(2), Some('c'));
        assert_eq!(src.peek_char(3), None);
    }

    #[test]
    fn eof() {
        let input = "";
        let mut src = SourceInput::new(input);
        assert!(src.is_eof());
        assert_eq!(src.next_char(), None);
    }
}
