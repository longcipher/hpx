use crate::css_parser::{source::SourceLocation, token::Token};

/// A parsed CSS stylesheet.
#[derive(Debug, Clone)]
pub struct Stylesheet<'a> {
    pub rules: Vec<Rule<'a>>,
    pub loc: SourceLocation,
}

/// A CSS rule: either a qualified rule (selector + block) or an at-rule.
#[derive(Debug, Clone)]
pub enum Rule<'a> {
    Qualified(QualifiedRule<'a>),
    At(AtRule<'a>),
}

/// A qualified rule: `selector { declarations; nested-rules }`.
#[derive(Debug, Clone)]
pub struct QualifiedRule<'a> {
    pub prelude: Vec<ComponentValue<'a>>,
    pub declarations: Vec<Declaration<'a>>,
    pub rules: Vec<Rule<'a>>,
    pub loc: SourceLocation,
}

/// An at-rule: `@name prelude { block }` or `@name prelude;`.
#[derive(Debug, Clone)]
pub struct AtRule<'a> {
    pub name: &'a str,
    pub prelude: Vec<ComponentValue<'a>>,
    pub block: Option<Block<'a>>,
    pub loc: SourceLocation,
}

/// The contents of an at-rule block.
#[derive(Debug, Clone)]
pub enum Block<'a> {
    RuleList(Vec<Rule<'a>>),
    DeclarationBlock {
        declarations: Vec<Declaration<'a>>,
        rules: Vec<Rule<'a>>,
    },
}

/// A CSS declaration: `property: value !important?`.
#[derive(Debug, Clone)]
pub struct Declaration<'a> {
    pub name: &'a str,
    pub value: Vec<ComponentValue<'a>>,
    pub important: bool,
    pub loc: SourceLocation,
}

/// A CSS component value.
#[derive(Debug, Clone)]
pub enum ComponentValue<'a> {
    Token(Token<'a>),
    Function(CssFunction<'a>),
    SimpleBlock(SimpleBlock<'a>),
}

/// A CSS function: `name(arg1, arg2, ...)`.
#[derive(Debug, Clone)]
pub struct CssFunction<'a> {
    pub name: &'a str,
    pub arguments: Vec<ComponentValue<'a>>,
    pub loc: SourceLocation,
}

/// A simple block delimited by `{}`, `[]`, or `()`.
#[derive(Debug, Clone)]
pub struct SimpleBlock<'a> {
    pub token: char,
    pub value: Vec<ComponentValue<'a>>,
    pub loc: SourceLocation,
}
