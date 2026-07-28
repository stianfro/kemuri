use std::time::Duration;

use async_trait::async_trait;
use kemuri_core::{ProbeKind, SampleClassification, SampleOutcome};
use serde::{Deserialize, Serialize};

use crate::{Probe, ProbeExecutionError, ProbeRound, ResolvedCheck, RoundContext, SampleResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticProbeConfig {
    pub outcome: SampleOutcome,
    pub latency: Duration,
    pub classification: SampleClassification,
}

impl Default for SyntheticProbeConfig {
    fn default() -> Self {
        Self {
            outcome: SampleOutcome::Success,
            latency: Duration::from_millis(10),
            classification: SampleClassification::HealthyResponse,
        }
    }
}

pub struct SyntheticProbe {
    config: SyntheticProbeConfig,
}

impl SyntheticProbe {
    pub fn new(config: SyntheticProbeConfig) -> Self {
        Self { config }
    }

    pub fn success(latency: Duration) -> Self {
        Self {
            config: SyntheticProbeConfig {
                outcome: SampleOutcome::Success,
                latency,
                classification: SampleClassification::HealthyResponse,
            },
        }
    }

    pub fn timeout() -> Self {
        Self {
            config: SyntheticProbeConfig {
                outcome: SampleOutcome::Timeout,
                latency: Duration::from_secs(5),
                classification: SampleClassification::MeasurementLoss,
            },
        }
    }
}

#[async_trait]
impl Probe for SyntheticProbe {
    fn kind(&self) -> ProbeKind {
        ProbeKind::Http
    }

    async fn execute_round(
        &self,
        _context: RoundContext,
        check: ResolvedCheck,
    ) -> Result<ProbeRound, ProbeExecutionError> {
        tokio::time::sleep(self.config.latency.min(check.timeout)).await;
        Ok(ProbeRound {
            check_id: check.check_id,
            results: vec![SampleResult {
                outcome: self.config.outcome,
                latency: Some(self.config.latency),
                detail: None,
                metadata: None,
            }],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kemuri_core::{CheckId, ObserverId, ProfileId, TargetId};
    use std::collections::HashMap;

    fn make_check() -> ResolvedCheck {
        ResolvedCheck {
            check_id: CheckId::new("test-check").unwrap(),
            target_id: TargetId::new("test-target").unwrap(),
            profile_id: ProfileId::new("test-profile").unwrap(),
            address: "synthetic".to_owned(),
            probe_kind: ProbeKind::Http,
            timeout: Duration::from_secs(5),
            sample_count: 1,
            params: HashMap::new(),
        }
    }

    fn make_context() -> RoundContext {
        RoundContext {
            observer_id: ObserverId::new("test-observer").unwrap(),
            scheduled_at: Duration::from_secs(0),
            deadline: Duration::from_secs(10),
        }
    }

    #[tokio::test]
    async fn synthetic_success_probe() {
        let probe = SyntheticProbe::success(Duration::from_millis(1));
        let result = probe
            .execute_round(make_context(), make_check())
            .await
            .unwrap();
        assert_eq!(result.results.len(), 1);
        assert_eq!(result.results[0].outcome, SampleOutcome::Success);
    }

    #[tokio::test]
    async fn synthetic_timeout_probe() {
        let probe = SyntheticProbe::timeout();
        let result = probe
            .execute_round(make_context(), make_check())
            .await
            .unwrap();
        assert_eq!(result.results[0].outcome, SampleOutcome::Timeout);
    }

    #[tokio::test]
    async fn synthetic_deterministic() {
        let probe = SyntheticProbe::success(Duration::from_millis(1));
        let r1 = probe
            .execute_round(make_context(), make_check())
            .await
            .unwrap();
        let r2 = probe
            .execute_round(make_context(), make_check())
            .await
            .unwrap();
        assert_eq!(r1.results[0].outcome, r2.results[0].outcome);
    }

    #[tokio::test]
    async fn synthetic_custom_config() {
        let probe = SyntheticProbe::new(SyntheticProbeConfig {
            outcome: SampleOutcome::ConnectionRefused,
            latency: Duration::from_millis(5),
            classification: SampleClassification::UnhealthyResponse,
        });
        let result = probe
            .execute_round(make_context(), make_check())
            .await
            .unwrap();
        assert_eq!(result.results[0].outcome, SampleOutcome::ConnectionRefused);
    }
}
