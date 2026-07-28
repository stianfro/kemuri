use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SampleOutcome {
    Success,
    Timeout,
    DnsError,
    NetworkUnreachable,
    ConnectionRefused,
    ConnectionReset,
    TlsError,
    ProtocolError,
    UnexpectedResponse,
    PermissionDenied,
    Cancelled,
    InternalError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundExecutionStatus {
    Complete,
    Partial,
    SkippedOverlap,
    SkippedBackpressure,
    Cancelled,
    InternalError,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_outcome_serde() {
        let outcomes = [
            SampleOutcome::Success,
            SampleOutcome::Timeout,
            SampleOutcome::ConnectionRefused,
            SampleOutcome::InternalError,
        ];
        for outcome in outcomes {
            let yaml = serde_yaml::to_string(&outcome).unwrap();
            let parsed: SampleOutcome = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(outcome, parsed);
        }
    }

    #[test]
    fn round_status_serde() {
        let statuses = [
            RoundExecutionStatus::Complete,
            RoundExecutionStatus::Partial,
            RoundExecutionStatus::Cancelled,
        ];
        for status in statuses {
            let yaml = serde_yaml::to_string(&status).unwrap();
            let parsed: RoundExecutionStatus = serde_yaml::from_str(&yaml).unwrap();
            assert_eq!(status, parsed);
        }
    }
}
