use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct AbiEntry {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub inputs: Vec<AbiParam>,
    #[serde(default)]
    pub outputs: Vec<AbiParam>,
    #[serde(default, rename = "stateMutability")]
    pub state_mutability: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AbiParam {
    #[serde(default)]
    pub name: String,
    #[serde(rename = "type")]
    pub ty: String,
    #[serde(default)]
    pub components: Vec<AbiParam>,
}

impl AbiEntry {
    pub fn is(&self, kind: &str) -> bool {
        self.kind == kind
    }
}
