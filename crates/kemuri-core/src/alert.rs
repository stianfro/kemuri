use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertState {
    Normal,
    PendingFire,
    Firing,
    PendingClear,
}

impl std::fmt::Display for AlertState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Normal => write!(f, "normal"),
            Self::PendingFire => write!(f, "pending_fire"),
            Self::Firing => write!(f, "firing"),
            Self::PendingClear => write!(f, "pending_clear"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertEventKind {
    Firing,
    Resolved,
}

impl std::fmt::Display for AlertEventKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Firing => write!(f, "firing"),
            Self::Resolved => write!(f, "resolved"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alert_state_serde_roundtrip() {
        let variants = [
            AlertState::Normal,
            AlertState::PendingFire,
            AlertState::Firing,
            AlertState::PendingClear,
        ];
        for v in variants {
            let yaml = serde_yaml::to_string(&v).unwrap();
            let parsed: AlertState = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(v, parsed);
        }
    }

    #[test]
    fn alert_event_kind_serde_roundtrip() {
        let variants = [AlertEventKind::Firing, AlertEventKind::Resolved];
        for v in variants {
            let yaml = serde_yaml::to_string(&v).unwrap();
            let parsed: AlertEventKind = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(v, parsed);
        }
    }

    #[test]
    fn alert_state_display() {
        assert_eq!(AlertState::Normal.to_string(), "normal");
        assert_eq!(AlertState::PendingFire.to_string(), "pending_fire");
        assert_eq!(AlertState::Firing.to_string(), "firing");
        assert_eq!(AlertState::PendingClear.to_string(), "pending_clear");
    }

    #[test]
    fn alert_event_kind_display() {
        assert_eq!(AlertEventKind::Firing.to_string(), "firing");
        assert_eq!(AlertEventKind::Resolved.to_string(), "resolved");
    }
}
