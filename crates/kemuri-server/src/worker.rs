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
    fatal_tx: Option<mpsc::Sender<&'static str>>,
}

impl WorkerPool {
    pub fn new(registry: Arc<ProbeRegistry>, num_workers: usize) -> Self {
        Self {
            registry,
            num_workers,
            fatal_tx: None,
        }
    }

    pub fn with_fatal_channel(mut self, sender: mpsc::Sender<&'static str>) -> Self {
        self.fatal_tx = Some(sender);
        self
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
            let fatal_tx = self.fatal_tx.clone();

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
                            break;
                        }
                    };

                    let key = (job.target_id.clone(), job.check_id.clone());
                    let result = execute_round(&registry, &job).await;

                    {
                        let mut running = running_rounds.lock().await;
                        running.remove(&key);
                    }

                    if result_tx.send(result).await.is_err() {
                        tracing::debug!(worker_id, "worker shutting down: result channel closed");
                        break;
                    }
                }
                if let Some(sender) = fatal_tx {
                    let _ = sender.try_send("probe_worker");
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

    let params = probe_params(&job.check.probe_params);

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
                    latency: None,
                    detail: None,
                    metadata: None,
                }],
                configured_samples: sample_count,
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
                    latency: None,
                    detail: None,
                    metadata: None,
                }],
                configured_samples: sample_count,
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
        configured_samples: sample_count,
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

pub fn probe_params(params: &ResolvedProbeParams) -> HashMap<String, String> {
    let mut values = HashMap::new();
    match params {
        ResolvedProbeParams::Icmp(params) => {
            values.insert("address_family".to_owned(), params.address_family.clone());
            values.insert("payload_size".to_owned(), params.payload_size.to_string());
            if let Some(source_address) = &params.source_address {
                values.insert("source_address".to_owned(), source_address.clone());
            }
        }
        ResolvedProbeParams::Http(params) => {
            values.insert("url".to_owned(), params.url.clone());
            if let Some(method) = &params.method {
                values.insert("method".to_owned(), method.clone());
            }
            if let Some(status) = params.expected_status {
                values.insert("expected_status".to_owned(), status.to_string());
            }
            if let Some((start, end)) = params.expected_status_range {
                values.insert("expected_status_range".to_owned(), format!("{start}-{end}"));
            }
            if let Some(body) = &params.body {
                values.insert("body".to_owned(), body.clone());
            }
            values.insert(
                "follow_redirects".to_owned(),
                params.follow_redirects.to_string(),
            );
            values.insert(
                "max_redirect_count".to_owned(),
                params.max_redirect_count.to_string(),
            );
            values.insert("connection_mode".to_owned(), params.connection_mode.clone());
            values.insert("measure_until".to_owned(), params.measure_until.clone());
            values.insert("tls_validate".to_owned(), params.tls_validate.to_string());
            if let Some(user_agent) = &params.user_agent {
                values.insert("user_agent".to_owned(), user_agent.clone());
            }
            if !params.root_certificates.is_empty()
                && let Ok(certificates) = serde_json::to_string(&params.root_certificates)
            {
                values.insert("root_certificates".to_owned(), certificates);
            }
            if !params.headers.is_empty()
                && let Ok(headers) = serde_json::to_string(&params.headers)
            {
                values.insert("headers".to_owned(), headers);
            }
        }
        ResolvedProbeParams::Tcp(params) => {
            values.insert("host".to_owned(), params.host.clone());
            values.insert("port".to_owned(), params.port.to_string());
            values.insert("address_family".to_owned(), params.address_family.clone());
            if let Some(source_address) = &params.source_address {
                values.insert("source_address".to_owned(), source_address.clone());
            }
            if let Some(tls) = &params.tls
                && let Ok(tls) = serde_json::to_string(tls)
            {
                values.insert("tls".to_owned(), tls);
            }
        }
        ResolvedProbeParams::Dns(params) => {
            values.insert("name".to_owned(), params.domain.clone());
            if let Some(record_type) = &params.record_type {
                values.insert("record_type".to_owned(), record_type.clone());
            }
            if let Some(server) = &params.resolver {
                values.insert("server".to_owned(), server.clone());
            }
            values.insert("protocol".to_owned(), params.protocol.clone());
            values.insert("expected_rcode".to_owned(), params.expected_rcode.clone());
            values.insert(
                "require_answer".to_owned(),
                params.require_answer.to_string(),
            );
        }
    }
    values
}
