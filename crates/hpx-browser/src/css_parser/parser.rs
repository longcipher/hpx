use crate::css_parser::{
    ast::*,
    error::ParseError,
    source::SourceLocation,
    token::{Token, TokenKind},
    tokenizer::Tokenizer,
};

/// CSS parser. Buffers all tokens for nesting disambiguation.
pub struct Parser<'a> {
    tokens: Vec<Token<'a>>,
    pos: usize,
    errors: Vec<ParseError>,
}

impl<'a> Parser<'a> {
    fn new(input: &'a str) -> Self {
        let tokens: Vec<Token<'a>> = {
            let mut tokenizer = Tokenizer::new(input);
            let mut tokens = Vec::new();
            loop {
                let token = tokenizer.next_token();
                let is_eof = token.kind == TokenKind::Eof;
                tokens.push(token);
                if is_eof {
                    break;
                }
            }
            tokens
        };

        Self {
            tokens,
            pos: 0,
            errors: Vec::new(),
        }
    }

    pub fn parse_stylesheet(input: &'a str) -> (Stylesheet<'a>, Vec<ParseError>) {
        let mut parser = Self::new(input);
        let loc = parser.current_location();
        let rules = parser.consume_rule_list(true);
        let errors = parser.errors;
        (Stylesheet { rules, loc }, errors)
    }

    pub fn parse_declaration_list(input: &'a str) -> (Vec<Declaration<'a>>, Vec<ParseError>) {
        let mut parser = Self::new(input);
        let (declarations, _rules) = parser.consume_block_contents();
        let errors = parser.errors;
        (declarations, errors)
    }

    fn current(&self) -> &Token<'a> {
        &self.tokens[self.pos.min(self.tokens.len() - 1)]
    }

    fn current_kind(&self) -> &TokenKind<'a> {
        &self.current().kind
    }

    fn current_location(&self) -> SourceLocation {
        self.current().loc
    }

    fn advance(&mut self) -> &Token<'a> {
        let token = &self.tokens[self.pos.min(self.tokens.len() - 1)];
        if self.pos < self.tokens.len() {
            self.pos += 1;
        }
        token
    }

    fn is_eof(&self) -> bool {
        matches!(self.current_kind(), TokenKind::Eof)
    }

    fn skip_whitespace(&mut self) {
        while matches!(self.current_kind(), TokenKind::Whitespace) {
            self.advance();
        }
    }

    fn consume_rule_list(&mut self, top_level: bool) -> Vec<Rule<'a>> {
        let mut rules = Vec::new();
        loop {
            self.skip_whitespace();
            match self.current_kind() {
                TokenKind::Eof => return rules,
                TokenKind::CloseCurly => return rules,
                TokenKind::AtKeyword(_) => {
                    if let Some(rule) = self.consume_at_rule() {
                        rules.push(Rule::At(rule));
                    }
                }
                TokenKind::Cdo | TokenKind::Cdc if top_level => {
                    self.advance();
                }
                _ => {
                    if let Some(rule) = self.consume_qualified_rule() {
                        rules.push(Rule::Qualified(rule));
                    }
                }
            }
        }
    }

    fn consume_at_rule(&mut self) -> Option<AtRule<'a>> {
        let loc = self.current_location();
        let name = match self.current_kind() {
            TokenKind::AtKeyword(n) => *n,
            _ => return None,
        };
        self.advance();

        let mut prelude = Vec::new();

        loop {
            match self.current_kind() {
                TokenKind::Semicolon => {
                    self.advance();
                    return Some(AtRule {
                        name,
                        prelude,
                        block: None,
                        loc,
                    });
                }
                TokenKind::Eof => {
                    return Some(AtRule {
                        name,
                        prelude,
                        block: None,
                        loc,
                    });
                }
                TokenKind::OpenCurly => {
                    self.advance();
                    let block = self.consume_at_rule_block(name);
                    if matches!(self.current_kind(), TokenKind::CloseCurly) {
                        self.advance();
                    }
                    return Some(AtRule {
                        name,
                        prelude,
                        block: Some(block),
                        loc,
                    });
                }
                _ => {
                    prelude.push(self.consume_component_value());
                }
            }
        }
    }

    fn consume_at_rule_block(&mut self, name: &str) -> Block<'a> {
        let is_rule_list = matches!(
            name.to_ascii_lowercase().as_str(),
            "media" | "supports" | "layer" | "container" | "scope" | "document" | "keyframes"
        );

        if is_rule_list {
            Block::RuleList(self.consume_rule_list(false))
        } else {
            let (declarations, rules) = self.consume_block_contents();
            Block::DeclarationBlock {
                declarations,
                rules,
            }
        }
    }

    fn consume_qualified_rule(&mut self) -> Option<QualifiedRule<'a>> {
        let loc = self.current_location();
        let mut prelude = Vec::new();

        loop {
            match self.current_kind() {
                TokenKind::Eof => {
                    self.errors.push(ParseError::UnexpectedEof {
                        loc: self.current_location(),
                    });
                    return None;
                }
                TokenKind::OpenCurly => {
                    self.advance();
                    let (declarations, rules) = self.consume_block_contents();
                    if matches!(self.current_kind(), TokenKind::CloseCurly) {
                        self.advance();
                    }
                    return Some(QualifiedRule {
                        prelude,
                        declarations,
                        rules,
                        loc,
                    });
                }
                _ => {
                    prelude.push(self.consume_component_value());
                }
            }
        }
    }

    fn consume_block_contents(&mut self) -> (Vec<Declaration<'a>>, Vec<Rule<'a>>) {
        let mut declarations = Vec::new();
        let mut rules = Vec::new();

        loop {
            self.skip_whitespace();
            match self.current_kind() {
                TokenKind::Eof | TokenKind::CloseCurly => {
                    return (declarations, rules);
                }
                TokenKind::Semicolon => {
                    self.advance();
                }
                TokenKind::AtKeyword(_) => {
                    if let Some(at_rule) = self.consume_at_rule() {
                        rules.push(Rule::At(at_rule));
                    }
                }
                TokenKind::Ident(_) => {
                    let saved_pos = self.pos;
                    if let Some(decl) = self.try_consume_declaration() {
                        declarations.push(decl);
                    } else {
                        self.pos = saved_pos;
                        if let Some(rule) = self.consume_qualified_rule() {
                            rules.push(Rule::Qualified(rule));
                        }
                    }
                }
                _ => {
                    if let Some(rule) = self.consume_qualified_rule() {
                        rules.push(Rule::Qualified(rule));
                    }
                }
            }
        }
    }

    fn try_consume_declaration(&mut self) -> Option<Declaration<'a>> {
        let loc = self.current_location();

        let name = match self.current_kind() {
            TokenKind::Ident(n) => *n,
            _ => return None,
        };
        self.advance();

        self.skip_whitespace();

        if !matches!(self.current_kind(), TokenKind::Colon) {
            return None;
        }
        self.advance();

        self.skip_whitespace();

        let mut value = Vec::new();
        loop {
            match self.current_kind() {
                TokenKind::Semicolon => {
                    self.advance();
                    break;
                }
                TokenKind::CloseCurly | TokenKind::Eof => {
                    break;
                }
                _ => {
                    value.push(self.consume_component_value());
                }
            }
        }

        let important = check_and_strip_important(&mut value);

        Some(Declaration {
            name,
            value,
            important,
            loc,
        })
    }

    fn consume_component_value(&mut self) -> ComponentValue<'a> {
        match self.current_kind() {
            TokenKind::OpenCurly | TokenKind::OpenSquare | TokenKind::OpenParen => {
                ComponentValue::SimpleBlock(self.consume_simple_block())
            }
            TokenKind::Function(_) => ComponentValue::Function(self.consume_function()),
            _ => {
                let token = self.advance().clone();
                ComponentValue::Token(token)
            }
        }
    }

    fn consume_simple_block(&mut self) -> SimpleBlock<'a> {
        let loc = self.current_location();
        let opening = match self.current_kind() {
            TokenKind::OpenCurly => '{',
            TokenKind::OpenSquare => '[',
            TokenKind::OpenParen => '(',
            _ => unreachable!(),
        };
        let closing = match opening {
            '{' => TokenKind::CloseCurly,
            '[' => TokenKind::CloseSquare,
            '(' => TokenKind::CloseParen,
            _ => unreachable!(),
        };
        self.advance();

        let mut value = Vec::new();
        loop {
            if *self.current_kind() == closing {
                self.advance();
                return SimpleBlock {
                    token: opening,
                    value,
                    loc,
                };
            }
            if self.is_eof() {
                return SimpleBlock {
                    token: opening,
                    value,
                    loc,
                };
            }
            value.push(self.consume_component_value());
        }
    }

    fn consume_function(&mut self) -> CssFunction<'a> {
        let loc = self.current_location();
        let name = match self.current_kind() {
            TokenKind::Function(n) => *n,
            _ => unreachable!(),
        };
        self.advance();

        let mut arguments = Vec::new();
        loop {
            match self.current_kind() {
                TokenKind::CloseParen => {
                    self.advance();
                    return CssFunction {
                        name,
                        arguments,
                        loc,
                    };
                }
                TokenKind::Eof => {
                    return CssFunction {
                        name,
                        arguments,
                        loc,
                    };
                }
                _ => {
                    arguments.push(self.consume_component_value());
                }
            }
        }
    }
}

fn check_and_strip_important(value: &mut Vec<ComponentValue<'_>>) -> bool {
    let mut i = value.len();
    while i > 0 {
        i -= 1;
        match &value[i] {
            ComponentValue::Token(Token {
                kind: TokenKind::Whitespace,
                ..
            }) => continue,
            ComponentValue::Token(Token {
                kind: TokenKind::Ident(name),
                ..
            }) if name.eq_ignore_ascii_case("important") => {
                while i > 0 {
                    i -= 1;
                    match &value[i] {
                        ComponentValue::Token(Token {
                            kind: TokenKind::Whitespace,
                            ..
                        }) => continue,
                        ComponentValue::Token(Token {
                            kind: TokenKind::Delim('!'),
                            ..
                        }) => {
                            value.truncate(i);
                            while value
                                .last()
                                .map(|v| {
                                    matches!(
                                        v,
                                        ComponentValue::Token(Token {
                                            kind: TokenKind::Whitespace,
                                            ..
                                        })
                                    )
                                })
                                .unwrap_or(false)
                            {
                                value.pop();
                            }
                            return true;
                        }
                        _ => return false,
                    }
                }
                return false;
            }
            _ => return false,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_rule() {
        let (stylesheet, errors) = Parser::parse_stylesheet("h1 { color: red; }");
        assert!(errors.is_empty());
        assert_eq!(stylesheet.rules.len(), 1);
        match &stylesheet.rules[0] {
            Rule::Qualified(qr) => {
                assert_eq!(qr.declarations.len(), 1);
                assert_eq!(qr.declarations[0].name, "color");
                assert!(!qr.declarations[0].important);
            }
            _ => panic!("Expected qualified rule"),
        }
    }

    #[test]
    fn parse_important() {
        let (stylesheet, _) = Parser::parse_stylesheet("h1 { color: red !important; }");
        match &stylesheet.rules[0] {
            Rule::Qualified(qr) => {
                assert!(qr.declarations[0].important);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_at_rule_media() {
        let (stylesheet, errors) = Parser::parse_stylesheet("@media screen { h1 { color: red; } }");
        assert!(errors.is_empty());
        assert_eq!(stylesheet.rules.len(), 1);
        match &stylesheet.rules[0] {
            Rule::At(at) => {
                assert_eq!(at.name, "media");
                assert!(at.block.is_some());
            }
            _ => panic!("Expected at-rule"),
        }
    }

    #[test]
    fn parse_multiple_declarations() {
        let (stylesheet, _) =
            Parser::parse_stylesheet("h1 { color: red; font-size: 2em; margin: 0; }");
        match &stylesheet.rules[0] {
            Rule::Qualified(qr) => {
                assert_eq!(qr.declarations.len(), 3);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_declaration_list_inline() {
        let (decls, _) = Parser::parse_declaration_list("color: red; font-size: 16px");
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].name, "color");
        assert_eq!(decls[1].name, "font-size");
    }

    #[test]
    fn parse_nested_rule() {
        let (stylesheet, errors) =
            Parser::parse_stylesheet(".card { color: black; &:hover { color: blue; } }");
        assert!(errors.is_empty(), "errors: {:?}", errors);
        match &stylesheet.rules[0] {
            Rule::Qualified(qr) => {
                assert_eq!(qr.declarations.len(), 1);
                assert_eq!(qr.rules.len(), 1);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_function_in_value() {
        let (stylesheet, _) = Parser::parse_stylesheet("h1 { color: rgb(255, 0, 0); }");
        match &stylesheet.rules[0] {
            Rule::Qualified(qr) => {
                let has_fn = qr.declarations[0]
                    .value
                    .iter()
                    .any(|v| matches!(v, ComponentValue::Function(f) if f.name == "rgb"));
                assert!(has_fn);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn parse_empty_stylesheet() {
        let (stylesheet, errors) = Parser::parse_stylesheet("");
        assert!(errors.is_empty());
        assert!(stylesheet.rules.is_empty());
    }

    #[test]
    fn error_recovery_unclosed_rule() {
        let (stylesheet, _) = Parser::parse_stylesheet("h1 { color: red; h2 { font-size: 1em; }");
        assert!(!stylesheet.rules.is_empty());
    }
}
