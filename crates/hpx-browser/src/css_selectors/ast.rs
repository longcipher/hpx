/// A comma-separated list of complex selectors.
pub type SelectorList = Vec<Selector>;

/// A complete selector. Components are stored right-to-left for efficient matching.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector {
    pub(crate) components: Vec<Component>,
    pub(crate) specificity: Specificity,
}

impl Selector {
    pub fn new(components: Vec<Component>, specificity: Specificity) -> Self {
        Self {
            components,
            specificity,
        }
    }

    pub fn components(&self) -> &[Component] {
        &self.components
    }

    pub fn specificity(&self) -> Specificity {
        self.specificity
    }
}

/// A component of a selector.
#[derive(Debug, Clone, PartialEq)]
pub enum Component {
    Combinator(Combinator),
    Simple(SimpleSelector),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Combinator {
    Descendant,
    Child,
    NextSibling,
    SubsequentSibling,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SimpleSelector {
    Type(String),
    Universal,
    Id(String),
    Class(String),
    Attribute {
        name: String,
        operator: Option<AttributeOperator>,
        value: Option<String>,
        case_sensitivity: CaseSensitivity,
    },
    PseudoClass(PseudoClass),
    PseudoElement(PseudoElement),
    Nesting,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeOperator {
    Exact,
    Includes,
    DashMatch,
    Prefix,
    Suffix,
    Substring,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaseSensitivity {
    Default,
    CaseInsensitive,
    CaseSensitive,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PseudoClass {
    AnyLink,
    Link,
    Visited,
    Target,
    Hover,
    Active,
    Focus,
    FocusWithin,
    FocusVisible,
    Enabled,
    Disabled,
    ReadWrite,
    ReadOnly,
    Checked,
    Default,
    Indeterminate,
    Required,
    Optional,
    Valid,
    Invalid,
    InRange,
    OutOfRange,
    PlaceholderShown,
    Root,
    Empty,
    FirstChild,
    LastChild,
    OnlyChild,
    FirstOfType,
    LastOfType,
    OnlyOfType,
    NthChild(NthExpr, Option<SelectorList>),
    NthLastChild(NthExpr, Option<SelectorList>),
    NthOfType(NthExpr),
    NthLastOfType(NthExpr),
    Lang(Vec<String>),
    Is(SelectorList),
    Not(SelectorList),
    Where(SelectorList),
    Has(Vec<RelativeSelector>),
}

/// An+B expression for nth-child etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NthExpr {
    pub a: i32,
    pub b: i32,
}

impl NthExpr {
    pub fn matches(&self, index: i32) -> bool {
        if self.a == 0 {
            return index == self.b;
        }
        let diff = index - self.b;
        if self.a > 0 {
            diff >= 0 && diff % self.a == 0
        } else {
            diff <= 0 && diff % self.a == 0
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct RelativeSelector {
    pub combinator: Option<Combinator>,
    pub selector: Selector,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PseudoElement {
    Before,
    After,
    FirstLine,
    FirstLetter,
    Placeholder,
    Selection,
    Part(Vec<String>),
    Slotted(Box<Selector>),
    Custom(String),
}

/// Specificity as (a, b, c) per Selectors §17.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct Specificity {
    pub a: u32,
    pub b: u32,
    pub c: u32,
}

impl Specificity {
    pub fn new(a: u32, b: u32, c: u32) -> Self {
        Self { a, b, c }
    }

    pub fn max(self, other: Self) -> Self {
        if self >= other { self } else { other }
    }
}

impl std::ops::Add for Specificity {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            a: self.a + rhs.a,
            b: self.b + rhs.b,
            c: self.c + rhs.c,
        }
    }
}

impl std::ops::AddAssign for Specificity {
    fn add_assign(&mut self, rhs: Self) {
        self.a += rhs.a;
        self.b += rhs.b;
        self.c += rhs.c;
    }
}
