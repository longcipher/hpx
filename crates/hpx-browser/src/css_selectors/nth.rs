use crate::{
    css_parser::{Token, TokenKind},
    css_selectors::{ast::NthExpr, error::SelectorParseError},
};

pub fn parse_nth(tokens: &[Token<'_>]) -> Result<NthExpr, SelectorParseError> {
    let tokens: Vec<&Token<'_>> = tokens.iter().filter(|t| !t.kind.is_whitespace()).collect();

    if tokens.is_empty() {
        return Err(SelectorParseError::InvalidNth("empty expression".into()));
    }

    if tokens.len() == 1 {
        if let TokenKind::Ident(name) = &tokens[0].kind {
            match name.to_ascii_lowercase().as_str() {
                "odd" => return Ok(NthExpr { a: 2, b: 1 }),
                "even" => return Ok(NthExpr { a: 2, b: 0 }),
                "n" => return Ok(NthExpr { a: 1, b: 0 }),
                "-n" => return Ok(NthExpr { a: -1, b: 0 }),
                _ => {}
            }
        }
        if let TokenKind::Number {
            int_value: Some(v), ..
        } = &tokens[0].kind
        {
            return Ok(NthExpr { a: 0, b: *v as i32 });
        }
    }

    let mut repr = String::new();
    for t in &tokens {
        match &t.kind {
            TokenKind::Ident(s) => repr.push_str(s),
            TokenKind::Number {
                int_value: Some(v),
                has_sign,
                ..
            } => {
                if *has_sign && *v >= 0 {
                    repr.push('+');
                }
                repr.push_str(&v.to_string());
            }
            TokenKind::Number {
                value, has_sign, ..
            } => {
                if *has_sign && *value >= 0.0 {
                    repr.push('+');
                }
                repr.push_str(&(*value as i64).to_string());
            }
            TokenKind::Dimension {
                int_value: Some(v),
                unit,
                ..
            } => {
                repr.push_str(&v.to_string());
                repr.push_str(unit);
            }
            TokenKind::Delim('+') => repr.push('+'),
            TokenKind::Delim('-') => repr.push('-'),
            _ => {}
        }
    }

    parse_nth_string(&repr)
}

fn parse_nth_string(s: &str) -> Result<NthExpr, SelectorParseError> {
    let s = s.to_ascii_lowercase();
    let s = s.trim();

    match s {
        "odd" => return Ok(NthExpr { a: 2, b: 1 }),
        "even" => return Ok(NthExpr { a: 2, b: 0 }),
        _ => {}
    }

    let n_pos_opt = s
        .char_indices()
        .find(|&(i, c)| {
            c == 'n' && {
                if i > 0 {
                    let prev = s.as_bytes()[i - 1];
                    !prev.is_ascii_alphabetic()
                } else {
                    true
                }
            }
        })
        .map(|(i, _)| i);

    if let Some(n_pos) = n_pos_opt {
        let a_part = &s[..n_pos];
        let b_part = s[n_pos + 1..].trim();

        let a = match a_part {
            "" | "+" => 1,
            "-" => -1,
            _ => a_part
                .parse::<i32>()
                .map_err(|_| SelectorParseError::InvalidNth(s.to_string()))?,
        };

        let b = if b_part.is_empty() {
            0
        } else {
            b_part
                .replace(' ', "")
                .parse::<i32>()
                .map_err(|_| SelectorParseError::InvalidNth(s.to_string()))?
        };

        Ok(NthExpr { a, b })
    } else {
        let b = s
            .parse::<i32>()
            .map_err(|_| SelectorParseError::InvalidNth(s.to_string()))?;
        Ok(NthExpr { a: 0, b })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &str) -> NthExpr {
        parse_nth_string(input).unwrap()
    }

    #[test]
    fn odd() {
        assert_eq!(parse("odd"), NthExpr { a: 2, b: 1 });
    }

    #[test]
    fn even() {
        assert_eq!(parse("even"), NthExpr { a: 2, b: 0 });
    }

    #[test]
    fn plain_number() {
        assert_eq!(parse("5"), NthExpr { a: 0, b: 5 });
    }

    #[test]
    fn three_n_plus_one() {
        assert_eq!(parse("3n+1"), NthExpr { a: 3, b: 1 });
    }

    #[test]
    fn neg_n_plus_six() {
        assert_eq!(parse("-n+6"), NthExpr { a: -1, b: 6 });
    }

    #[test]
    fn nth_matches() {
        let expr = NthExpr { a: 2, b: 1 };
        assert!(expr.matches(1));
        assert!(!expr.matches(2));
        assert!(expr.matches(3));
    }
}
