use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKind {
    Icmp,
    Http,
    Tcp,
    Dns,
}

impl std::fmt::Display for ProbeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Icmp => write!(f, "icmp"),
            Self::Http => write!(f, "http"),
            Self::Tcp => write!(f, "tcp"),
            Self::Dns => write!(f, "dns"),
        }
    }
}

impl std::str::FromStr for ProbeKind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "icmp" => Ok(Self::Icmp),
            "http" => Ok(Self::Http),
            "tcp" => Ok(Self::Tcp),
            "dns" => Ok(Self::Dns),
            _ => Err(format!("unknown probe kind: {}", s)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serde_roundtrip() {
        let kinds = [
            ProbeKind::Icmp,
            ProbeKind::Http,
            ProbeKind::Tcp,
            ProbeKind::Dns,
        ];
        for kind in kinds {
            let yaml = serde_yaml::to_string(&kind).unwrap();
            let parsed: ProbeKind = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(kind, parsed);
        }
    }
}
