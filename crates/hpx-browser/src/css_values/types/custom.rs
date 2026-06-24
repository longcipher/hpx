#[derive(Debug, Clone, PartialEq)]
pub struct VarReference {
    pub name: String,
    pub fallback: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnvReference {
    pub name: String,
    pub fallback: Option<String>,
}
