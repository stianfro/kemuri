use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Unknown,
    Healthy,
    Degraded,
    Down,
    NoData,
    Disabled,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let variants = [
            CheckState::Unknown,
            CheckState::Healthy,
            CheckState::Degraded,
            CheckState::Down,
            CheckState::NoData,
            CheckState::Disabled,
        ];
        for v in variants {
            let yaml = serde_yaml::to_string(&v).unwrap();
            let parsed: CheckState = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(v, parsed);
        }
    }
}
