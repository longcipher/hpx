//! Minimal CSS `font` shorthand parser for Canvas 2D `ctx.font = ...`.

#[derive(Debug, Clone, PartialEq)]
pub struct ParsedFont {
    pub weight: u16,
    pub italic: bool,
    pub size_px: f32,
    pub families: Vec<String>,
}

impl ParsedFont {
    /// Default matches Canvas 2D spec: `10px sans-serif`.
    pub fn default_font() -> Self {
        Self {
            weight: 400,
            italic: false,
            size_px: 10.0,
            families: vec!["sans-serif".to_string()],
        }
    }

    /// Parse a `ctx.font` string.
    pub fn parse(input: &str) -> Option<Self> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return None;
        }

        let mut parsed = Self::default_font();
        let (prefix_tokens, family_segment) = split_family_list(trimmed);

        let mut size_found = false;
        for tok in &prefix_tokens {
            if size_found {
                continue;
            }
            if let Some(size) = parse_size_token(tok) {
                parsed.size_px = size;
                size_found = true;
                continue;
            }
            let lower = tok.to_ascii_lowercase();
            match lower.as_str() {
                "italic" | "oblique" => parsed.italic = true,
                "normal" => {}
                "bold" | "bolder" => parsed.weight = 700,
                "lighter" => parsed.weight = 300,
                _ => {
                    if let Ok(w) = lower.parse::<u16>() {
                        if (100..=900).contains(&w) {
                            parsed.weight = w;
                        }
                    }
                }
            }
        }

        if !size_found {
            return None;
        }

        let mut families = parse_family_list(&family_segment);
        if families.is_empty() {
            families.push("sans-serif".to_string());
        }
        parsed.families = families;
        Some(parsed)
    }
}

fn parse_size_token(tok: &str) -> Option<f32> {
    let tok = tok.split('/').next()?;
    if tok.is_empty() {
        return None;
    }
    let unit_start = tok.find(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')?;
    let (num_str, unit) = tok.split_at(unit_start);
    let num: f32 = num_str.parse().ok()?;
    match unit {
        "px" => Some(num),
        "pt" => Some(num * 96.0 / 72.0),
        "em" | "rem" => Some(num * 16.0),
        "%" => Some(num * 16.0 / 100.0),
        _ => None,
    }
}

fn split_family_list(input: &str) -> (Vec<String>, String) {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    let mut token_start = 0usize;

    for (i, c) in input.char_indices() {
        if let Some(q) = in_quote {
            if c == q {
                in_quote = None;
            } else {
                current.push(c);
            }
            continue;
        }
        match c {
            '"' | '\'' => {
                in_quote = Some(c);
            }
            ' ' | '\t' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                token_start = i + 1;
            }
            ',' => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
                let family_segment = input[token_start..].to_string();
                return (tokens, family_segment);
            }
            _ => current.push(c),
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }

    let mut prefix = Vec::new();
    let mut family_tokens: Vec<String> = Vec::new();
    let mut seen_size = false;
    for tok in tokens {
        if seen_size {
            family_tokens.push(tok);
        } else if parse_size_token(&tok).is_some() {
            prefix.push(tok);
            seen_size = true;
        } else {
            prefix.push(tok);
        }
    }
    let family_segment = family_tokens.join(" ");
    (prefix, family_segment)
}

fn parse_family_list(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_quote: Option<char> = None;
    for c in s.chars() {
        match in_quote {
            Some(q) => {
                if c == q {
                    in_quote = None;
                } else {
                    current.push(c);
                }
            }
            None => match c {
                '"' | '\'' => in_quote = Some(c),
                ',' => {
                    let trimmed = current.trim().to_string();
                    if !trimmed.is_empty() {
                        out.push(trimmed);
                    }
                    current.clear();
                }
                _ => current.push(c),
            },
        }
    }
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        out.push(trimmed);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple() {
        let p = ParsedFont::parse("14px Arial").expect("should parse");
        assert_eq!(p.size_px, 14.0);
        assert_eq!(p.weight, 400);
        assert!(!p.italic);
        assert_eq!(p.families, vec!["Arial".to_string()]);
    }

    #[test]
    fn parses_bold_italic() {
        let p = ParsedFont::parse("bold italic 16px 'Times New Roman'").expect("should parse");
        assert_eq!(p.weight, 700);
        assert!(p.italic);
        assert_eq!(p.size_px, 16.0);
        assert_eq!(p.families, vec!["Times New Roman".to_string()]);
    }

    #[test]
    fn parses_multi_family() {
        let p = ParsedFont::parse("14px Arial, Helvetica, sans-serif").expect("should parse");
        assert_eq!(p.size_px, 14.0);
        assert_eq!(
            p.families,
            vec![
                "Arial".to_string(),
                "Helvetica".to_string(),
                "sans-serif".to_string()
            ]
        );
    }

    #[test]
    fn missing_size_is_error() {
        assert!(ParsedFont::parse("bold italic Arial").is_none());
    }

    #[test]
    fn empty_is_error() {
        assert!(ParsedFont::parse("").is_none());
    }
}
