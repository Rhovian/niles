use serde::Deserialize;

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Deserialize,
    clap::ValueEnum,
    schemars::JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Frontier,
    Standard,
    Economy,
    Mechanical,
}

impl Tier {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Frontier => "frontier",
            Self::Standard => "standard",
            Self::Economy => "economy",
            Self::Mechanical => "mechanical",
        }
    }
}
