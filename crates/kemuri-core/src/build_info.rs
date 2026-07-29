use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildInfo {
    pub version: String,
    pub git_hash: String,
    pub build_timestamp_ms: i64,
    pub target: String,
}
