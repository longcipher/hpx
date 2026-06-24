//! CSS Selectors Level 4 parser and matching engine.

pub mod ast;
pub mod element;
pub mod error;
pub mod matching;
pub mod nth;
pub mod parser;
pub mod specificity;

pub use ast::*;
pub use element::Element;
pub use error::SelectorParseError;
pub use matching::{matches_any, matches_selector, query_selector, query_selector_all};
pub use parser::{parse_selector_list, parse_selector_list_forgiving};
pub use specificity::compute_specificity;
