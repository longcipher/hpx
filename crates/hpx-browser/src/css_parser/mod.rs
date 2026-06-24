//! CSS Syntax Level 3 tokenizer and parser with CSS Nesting support.

pub mod ast;
pub mod error;
pub mod parser;
pub mod source;
pub mod token;
pub mod tokenizer;

pub use ast::*;
pub use error::ParseError;
pub use parser::Parser;
pub use source::SourceLocation;
pub use token::{Token, TokenKind, resolve_escapes};
pub use tokenizer::Tokenizer;

/// Parse a complete CSS stylesheet.
pub fn parse_stylesheet(input: &str) -> (Stylesheet<'_>, Vec<ParseError>) {
    Parser::parse_stylesheet(input)
}

/// Parse a list of CSS declarations (e.g., from a `style` attribute).
pub fn parse_declaration_list(input: &str) -> (Vec<Declaration<'_>>, Vec<ParseError>) {
    Parser::parse_declaration_list(input)
}
