use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use kemuri_config::{ResolvedCheckDef, ResolvedProbeParams};
use kemuri_core::{CheckId, SampleOutcome, TargetId};
use kemuri_probes::{ResolvedCheck, RoundContext, SampleResult};
use tokio::sync::mpsc;

use crate::probe_registry::ProbeRegistry;
use crate::writer::RoundResult;

pub struct RoundJob {
    pub target_id: TargetId,
    pub check_id: CheckId,
    pub check: ResolvedCheckDef,
    pub scheduled_at: DateTime<Utc>,
}

pub struct WorkerPool {
    registry: Arc<ProbeRegistry>,
    num_workers: usize,
}

impl WorkerPool {
    pub fn new(registry: Arc<ProbeRegistry>, num_workers: usize) -> Self {
        Self {
            registry,
            num_workers,
        }
    }

    pub fn start(
        &self,
        job_rx: mpsc::Receiver<RoundJob>,
        result_tx: mpsc::Sender<RoundResult>,
        running_rounds: Arc<tokio::sync::Mutex<std::collections::HashSet<(TargetId, CheckId)>>>,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        let mut handles = Vec::new();
        let job_rx = Arc::new(tokio::sync::Mutex::new(job_rx));

        for worker_id in 0..self.num_workers {
            let job_rx = job_rx.clone();
            let result_tx = result_tx.clone();
            let registry = self.registry.clone();
            let running_rounds = running_rounds.clone();

            let handle = tokio::spawn(async move {
                loop {
                    let job = {
                        let mut rx = job_rx.lock().await;
                        rx.recv().await
                    };

                    let job = match job {
                        Some(j) => j,
                        None => {
                            tracing::debug!(worker_id, "worker shutting down: channel closed");
                            return;
                        }
                    };

                    let key = (job.target_id.clone(), job.check_id.clone());
                    {
                        let mut running = running_rounds.lock().await;
                        running.insert(key.clone());
                    }

                    let result = execute_round(&registry, &job).await;

                    {
                        let mut running = running_rounds.lock().await;
                        running.remove(&key);
                    }

                    if result_tx.send(result).await.is_err() {
                        tracing::debug!(worker_id, "worker shutting down: result channel closed");
                        return;
                    }
                }
            });

            handles.push(handle);
        }

        handles
    }
}

async fn execute_round(registry: &ProbeRegistry, job: &RoundJob) -> RoundResult {
    let started_at = Utc::now();
    let probe = match registry.get(job.check.probe_kind) {
        Some(p) => p,
        None => {
            tracing::error!(
                check_id = %job.check_id,
                probe_kind = %job.check.probe_kind,
                "no probe registered for kind"
            );
            return RoundResult {
                target_id: job.target_id.clone(),
                check_id: job.check_id.clone(),
                scheduled_at: job.scheduled_at,
                started_at,
                finished_at: Utc::now(),
                execution_status: kemuri_core::RoundExecutionStatus::InternalError,
                stop_reason: Some("no_probe_registered".to_owned()),
                sample_results: vec![],
                configured_samples: 1,
            };
        }
    };

    let context = RoundContext {
        observer_id: kemuri_core::ObserverId::new("local")
            .unwrap_or_else(|_| kemuri_core::ObserverId::new("obs").unwrap()),
        scheduled_at: Duration::from_millis(0),
        deadline: job.check.timeout,
    };

    let sample_count = match &job.check.probe_params {
        ResolvedProbeParams::Icmp(p) => p.count,
        _ => 1,
    };

    let params = match &job.check.probe_params {
        ResolvedProbeParams::Tcp(p) => {
            let mut m = HashMap::new();
            m.insert("host".to_owned(), p.host.clone());
            m.insert("port".to_owned(), p.port.to_string());
            m
        }
        ResolvedProbeParams::Dns(p) => {
            let mut m = HashMap::new();
            m.insert("name".to_owned(), p.domain.clone());
            if let Some(ref rt) = p.record_type {
                m.insert("record_type".to_owned(), rt.clone());
            }
            if let Some(ref srv) = p.resolver {
                m.insert("server".to_owned(), srv.clone());
            }
            m
        }
        _ => HashMap::new(),
    };

    let resolved_check = ResolvedCheck {
        check_id: job.check.check_id.clone(),
        target_id: job.check.target_id.clone(),
        profile_id: job.check.profile_id.clone(),
        address: job.check.target_address.clone(),
        probe_kind: job.check.probe_kind,
        timeout: job.check.timeout,
        sample_count,
        params,
    };

    let round_result = match tokio::time::timeout(
        job.check.timeout,
        probe.execute_round(context, resolved_check),
    )
    .await
    {
        Ok(Ok(round)) => round,
        Ok(Err(e)) => {
            tracing::warn!(
                check_id = %job.check_id,
                error = %e,
                "probe execution error"
            );
            let finished_at = Utc::now();
            return RoundResult {
                target_id: job.target_id.clone(),
                check_id: job.check_id.clone(),
                scheduled_at: job.scheduled_at,
                started_at,
                finished_at,
                execution_status: kemuri_core::RoundExecutionStatus::InternalError,
                stop_reason: Some(e.to_string()),
                sample_results: vec![SampleResult {
                    outcome: SampleOutcome::InternalError,
                    latency: Some(
                        finished_at
                            .signed_duration_since(started_at)
                            .to_std()
                            .unwrap_or(Duration::ZERO),
                    ),
                    detail: None,
                    metadata: None,
                }],
                configured_samples: 1,
            };
        }
        Err(_) => {
            tracing::warn!(
                check_id = %job.check_id,
                "probe execution timed out"
            );
            let finished_at = Utc::now();
            return RoundResult {
                target_id: job.target_id.clone(),
                check_id: job.check_id.clone(),
                scheduled_at: job.scheduled_at,
                started_at,
                finished_at,
                execution_status: kemuri_core::RoundExecutionStatus::Complete,
                stop_reason: Some("timeout".to_owned()),
                sample_results: vec![SampleResult {
                    outcome: SampleOutcome::Timeout,
                    latency: Some(job.check.timeout),
                    detail: None,
                    metadata: None,
                }],
                configured_samples: 1,
            };
        }
    };

    let finished_at = Utc::now();
    let execution_status = classify_execution_status(&round_result.results);

    let round_duration = finished_at.signed_duration_since(started_at);
    metrics::histogram!("kemuri_probe_round_duration_ns",
        "probe_kind" => job.check.probe_kind.to_string())
    .record(round_duration.num_nanoseconds().unwrap_or(0) as f64);

    RoundResult {
        target_id: job.target_id.clone(),
        check_id: job.check_id.clone(),
        scheduled_at: job.scheduled_at,
        started_at,
        finished_at,
        execution_status,
        stop_reason: None,
        sample_results: round_result.results,
        configured_samples: 1,
    }
}

fn classify_execution_status(results: &[SampleResult]) -> kemuri_core::RoundExecutionStatus {
    if results.is_empty() {
        return kemuri_core::RoundExecutionStatus::Complete;
    }
    let has_loss = results.iter().any(|r| {
        r.outcome != SampleOutcome::Success && r.outcome != SampleOutcome::UnexpectedResponse
    });
    let has_success = results.iter().any(|r| {
        r.outcome == SampleOutcome::Success || r.outcome == SampleOutcome::UnexpectedResponse
    });
    if has_loss && has_success {
        kemuri_core::RoundExecutionStatus::Partial
    } else {
        kemuri_core::RoundExecutionStatus::Complete
    }
}
