use camino::Utf8PathBuf;

use super::kind::ArtifactKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SchemaStatus {
    Current(u64),
    Older(u64),
    Newer(u64),
    Invalid,
    Malformed,
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaObservation {
    pub(crate) kind: ArtifactKind,
    pub(crate) path: Utf8PathBuf,
    pub(crate) status: SchemaStatus,
}

impl SchemaStatus {
    pub(crate) fn is_problem(&self) -> bool {
        !matches!(self, SchemaStatus::Current(_))
    }

    pub(crate) fn summary(&self) -> String {
        match self {
            SchemaStatus::Current(schema) => format!("current schema {schema}"),
            SchemaStatus::Older(schema) => format!("older schema {schema}"),
            SchemaStatus::Newer(schema) => format!("newer schema {schema}"),
            SchemaStatus::Invalid => "invalid schema stamp".to_owned(),
            SchemaStatus::Malformed => "malformed artifact".to_owned(),
            SchemaStatus::Unreadable => "unreadable artifact".to_owned(),
        }
    }
}
