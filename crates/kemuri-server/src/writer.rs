use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use kemuri_core::{
    CheckId, RoundExecutionStatus, SampleClassification, SampleOutcome, TargetId, encode_samples,
};
use kemuri_probes::SampleResult;
use kemuri_storage::{
    CheckCurrentStateRepo, CheckRepo, InsertRound, StorageError, StorageManager, TargetRepo,
    UpsertCheckCurrentState,
};

use crate::events::SystemEvent;

pub struct RoundResult {
    pub target_id: TargetId,
    pub check_id: CheckId,
    pub scheduled_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub execution_status: RoundExecutionStatus,
    pub stop_reason: Option<String>,
    pub sample_results: Vec<SampleResult>,
    pub configured_samples: u32,
}

struct RoundStats {
    attempted: usize,
    latency_bearing: usize,
    healthy: usize,
    unhealthy: usize,
    measurement_loss: usize,
    min_lat: Option<Duration>,
    max_lat: Option<Duration>,
    sample_blob: Option<Vec<u8>>,
}

pub struct StorageWriter {
    storage: Arc<StorageManager>,
    observer_internal_id: i64,
    alert_tx: Option<tokio::sync::mpsc::Sender<crate::alerts::AlertNotification>>,
    event_tx: Option<tokio::sync::broadcast::Sender<SystemEvent>>,
}

impl StorageWriter {
    pub fn new(storage: Arc<StorageManager>, observer_internal_id: i64) -> Self {
        Self {
            storage,
            observer_internal_id,
            alert_tx: None,
            event_tx: None,
        }
    }

    pub fn with_alert_channel(
        mut self,
        alert_tx: tokio::sync::mpsc::Sender<crate::alerts::AlertNotification>,
    ) -> Self {
        self.alert_tx = Some(alert_tx);
        self
    }

    pub fn with_event_channel(
        mut self,
        event_tx: tokio::sync::broadcast::Sender<SystemEvent>,
    ) -> Self {
        self.event_tx = Some(event_tx);
        self
    }

    pub async fn run(&self, mut rx: tokio::sync::mpsc::Receiver<RoundResult>) {
        while let Some(result) = rx.recv().await {
            let start = std::time::Instant::now();
            if let Err(e) = self.write_round(result).await {
                if let Some(suppressed) = crate::failure_log::failure("writer", "database") {
                    tracing::error!(error = %e, suppressed, "failed to write round result");
                }
                metrics::counter!("kemuri_writer_errors").increment(1);
                metrics::counter!("kemuri_storage_writes_total", "result" => "error").increment(1);
            } else {
                if let Some(suppressed) = crate::failure_log::recovery("writer", "database") {
                    tracing::info!(
                        suppressed,
                        "storage writer recovered after repeated failures"
                    );
                }
                metrics::counter!("kemuri_storage_writes_total", "result" => "success")
                    .increment(1);
            }
            metrics::histogram!("kemuri_storage_write_duration_seconds")
                .record(start.elapsed().as_secs_f64());
        }
        tracing::info!("storage writer shutting down");
    }

    async fn write_round(&self, result: RoundResult) -> Result<(), StorageError> {
        let pool = self.storage.pool();

        let target_row = TargetRepo::get_by_target_id(pool, result.target_id.as_str())
            .await
            .map_err(StorageError::Db)?;

        let target_internal_id = match target_row {
            Some(t) => t.internal_id,
            None => {
                tracing::warn!(target_id = %result.target_id, "target not found in database");
                return Ok(());
            }
        };

        let check_row = CheckRepo::get(pool, target_internal_id, result.check_id.as_str())
            .await
            .map_err(StorageError::Db)?;

        let check_internal_id = match check_row {
            Some(c) if c.active => c.internal_id,
            _ => {
                tracing::warn!(
                    target_id = %result.target_id,
                    check_id = %result.check_id,
                    "check not found or inactive in database"
                );
                return Ok(());
            }
        };

        let stats = compute_round_stats(&result.sample_results);
        let median_lat = compute_median_latency(&result.sample_results);

        let execution_status_str = match result.execution_status {
            RoundExecutionStatus::Complete => "complete",
            RoundExecutionStatus::Partial => "partial",
            RoundExecutionStatus::SkippedOverlap => "skipped_overlap",
            RoundExecutionStatus::SkippedBackpressure => "skipped_backpressure",
            RoundExecutionStatus::Cancelled => "cancelled",
            RoundExecutionStatus::InternalError => "internal_error",
        };

        let outcome_summary = format_outcome_summary(&result.sample_results);

        let insert = InsertRound {
            check_internal_id,
            observer_internal_id: self.observer_internal_id,
            scheduled_at: result.scheduled_at.to_rfc3339(),
            started_at: Some(result.started_at.to_rfc3339()),
            finished_at: Some(result.finished_at.to_rfc3339()),
            execution_status: execution_status_str.to_owned(),
            stop_reason: result.stop_reason,
            configured_samples: result.configured_samples as i32,
            attempted_samples: stats.attempted as i32,
            latency_bearing_samples: stats.latency_bearing as i32,
            healthy_samples: stats.healthy as i32,
            unhealthy_samples: stats.unhealthy as i32,
            measurement_loss_samples: stats.measurement_loss as i32,
            min_latency_ns: stats.min_lat.map(|d| d.as_nanos() as i64),
            median_latency_ns: median_lat.map(|d| d.as_nanos() as i64),
            max_latency_ns: stats.max_lat.map(|d| d.as_nanos() as i64),
            sample_blob: stats.sample_blob,
            outcome_summary: Some(outcome_summary),
            config_generation: None,
            check_revision_id: None,
        };

        let _round_id = self.storage.write_round(insert).await?;

        let new_state = classify_state(&result.sample_results, result.execution_status);

        let old_state_row =
            CheckCurrentStateRepo::get(pool, check_internal_id, self.observer_internal_id)
                .await
                .map_err(StorageError::Db)?;
        let old_state = old_state_row
            .as_ref()
            .map(|r| r.state.clone())
            .unwrap_or_else(|| "no_data".to_owned());

        let last_latency = stats.min_lat.or(stats.max_lat);
        let total = (stats.healthy + stats.unhealthy + stats.measurement_loss) as f64;
        let ml_ratio = if total > 0.0 {
            stats.measurement_loss as f64 / total
        } else {
            0.0
        };
        let hf_ratio = if total > 0.0 {
            stats.unhealthy as f64 / total
        } else {
            0.0
        };

        let upsert = UpsertCheckCurrentState {
            check_internal_id,
            observer_internal_id: self.observer_internal_id,
            state: new_state.clone(),
            last_round_at: Some(result.scheduled_at.to_rfc3339()),
            last_latency_ns: last_latency.map(|d| d.as_nanos() as i64),
            last_measurement_loss_ratio: Some(ml_ratio),
            last_health_failure_ratio: Some(hf_ratio),
        };

        CheckCurrentStateRepo::upsert(pool, &upsert)
            .await
            .map_err(StorageError::Db)?;

        if let Some(ref alert_tx) = self.alert_tx {
            let notif = crate::alerts::AlertNotification {
                target_id: result.target_id.clone(),
                check_id: result.check_id.clone(),
                scheduled_at: result.scheduled_at,
            };
            let _ = alert_tx.try_send(notif);
        }

        if let Some(ref event_tx) = self.event_tx {
            let _ = event_tx.send(SystemEvent::round_completed(
                result.target_id.as_str(),
                result.check_id.as_str(),
            ));
            if old_state != new_state {
                let _ = event_tx.send(SystemEvent::check_state_changed(
                    result.target_id.as_str(),
                    result.check_id.as_str(),
                    &old_state,
                    &new_state,
                ));
            }
        }

        metrics::counter!("kemuri_probe_rounds_total",
            "probe_type" => result.sample_results.first().map(|_| "synthetic").unwrap_or("unknown"),
            "status" => execution_status_str)
        .increment(1);

        Ok(())
    }
}

fn compute_round_stats(results: &[SampleResult]) -> RoundStats {
    let mut attempted = 0;
    let mut latency_bearing = 0;
    let mut healthy = 0;
    let mut unhealthy = 0;
    let mut measurement_loss = 0;
    let mut min_lat: Option<Duration> = None;
    let mut max_lat: Option<Duration> = None;
    let mut records: Vec<kemuri_core::SampleRecord> = Vec::new();

    for (i, r) in results.iter().enumerate() {
        attempted += 1;
        let classification = classify_outcome(&r.outcome);
        match classification {
            SampleClassification::HealthyResponse => {
                healthy += 1;
                if r.latency.is_some() {
                    latency_bearing += 1;
                }
            }
            SampleClassification::UnhealthyResponse => {
                unhealthy += 1;
                if r.latency.is_some() {
                    latency_bearing += 1;
                }
            }
            SampleClassification::MeasurementLoss => {
                measurement_loss += 1;
            }
        }

        if let Some(lat) = r.latency {
            min_lat = Some(min_lat.map_or(lat, |m| m.min(lat)));
            max_lat = Some(max_lat.map_or(lat, |m| m.max(lat)));
        }

        records.push(kemuri_core::SampleRecord {
            sample_index: i as u16,
            offset_us: 0,
            outcome: r.outcome,
            classification,
            latency_ns: r.latency.map(|d| d.as_nanos() as u64),
            elapsed_ns: r.latency.map(|d| d.as_nanos() as u64),
            metadata: if let Some(ref meta) = r.metadata {
                Some(serde_json::to_vec(meta).unwrap_or_default())
            } else {
                r.detail.as_ref().map(|s| s.as_bytes().to_vec())
            },
        });
    }

    let sample_blob = if records.is_empty() {
        None
    } else {
        Some(encode_samples(&records))
    };

    RoundStats {
        attempted,
        latency_bearing,
        healthy,
        unhealthy,
        measurement_loss,
        min_lat,
        max_lat,
        sample_blob,
    }
}

fn compute_median_latency(results: &[SampleResult]) -> Option<Duration> {
    let mut latencies: Vec<Duration> = results.iter().filter_map(|r| r.latency).collect();
    if latencies.is_empty() {
        return None;
    }
    latencies.sort();
    let mid = latencies.len() / 2;
    if latencies.len().is_multiple_of(2) && mid > 0 {
        Some((latencies[mid - 1] + latencies[mid]) / 2)
    } else {
        Some(latencies[mid])
    }
}

fn classify_outcome(outcome: &SampleOutcome) -> SampleClassification {
    match outcome {
        SampleOutcome::Success => SampleClassification::HealthyResponse,
        SampleOutcome::UnexpectedResponse => SampleClassification::UnhealthyResponse,
        SampleOutcome::Timeout
        | SampleOutcome::DnsError
        | SampleOutcome::NetworkUnreachable
        | SampleOutcome::ConnectionRefused
        | SampleOutcome::ConnectionReset
        | SampleOutcome::TlsError
        | SampleOutcome::ProtocolError
        | SampleOutcome::PermissionDenied
        | SampleOutcome::Cancelled
        | SampleOutcome::InternalError => SampleClassification::MeasurementLoss,
    }
}

fn classify_state(results: &[SampleResult], execution_status: RoundExecutionStatus) -> String {
    match execution_status {
        RoundExecutionStatus::SkippedOverlap
        | RoundExecutionStatus::SkippedBackpressure
        | RoundExecutionStatus::Cancelled
        | RoundExecutionStatus::InternalError => "no_data",
        _ => {
            if results.is_empty() {
                return "no_data".to_owned();
            }
            let all_healthy = results
                .iter()
                .all(|r| classify_outcome(&r.outcome) == SampleClassification::HealthyResponse);
            let all_measurement_loss = results
                .iter()
                .all(|r| classify_outcome(&r.outcome) == SampleClassification::MeasurementLoss);

            if all_healthy {
                "healthy"
            } else if all_measurement_loss {
                "down"
            } else {
                "degraded"
            }
        }
    }
    .to_owned()
}

fn format_outcome_summary(results: &[SampleResult]) -> String {
    if results.is_empty() {
        return "no samples".to_owned();
    }
    let healthy = results
        .iter()
        .filter(|r| r.outcome == SampleOutcome::Success)
        .count();
    let total = results.len();
    format!("{}/{} healthy", healthy, total)
}
