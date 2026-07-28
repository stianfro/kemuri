use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use kemuri_core::{AlertEventKind, Clock, NotifierId};
use kemuri_storage::{
    AlertEventRepo, AlertStateRepo, CheckRepo, NotificationOutboxRepo, TargetRepo,
};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

use crate::alerts::compute_backoff;
use crate::notifiers::{NotificationPayload, Notifier};

const MAX_RETRY_ATTEMPTS: i64 = 10;
const BATCH_SIZE: i64 = 50;

pub struct NotificationWorker {
    pool: Arc<SqlitePool>,
    notifiers: Arc<std::sync::RwLock<HashMap<NotifierId, Arc<dyn Notifier>>>>,
    clock: Arc<dyn Clock>,
    public_url: Option<String>,
}

impl NotificationWorker {
    pub fn new(
        pool: Arc<SqlitePool>,
        notifiers: Arc<std::sync::RwLock<HashMap<NotifierId, Arc<dyn Notifier>>>>,
        clock: Arc<dyn Clock>,
        public_url: Option<String>,
    ) -> Self {
        Self {
            pool,
            notifiers,
            clock,
            public_url,
        }
    }

    pub async fn run(self, mut shutdown_rx: broadcast::Receiver<()>) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = shutdown_rx.recv() => {
                    tracing::info!("notification worker shutting down");
                    return;
                }
            }

            if let Err(e) = self.run_cycle().await {
                tracing::error!(error = %e, "notification worker cycle failed");
                metrics::counter!("kemuri_notification_worker_errors").increment(1);
            }
        }
    }

    async fn run_cycle(&self) -> Result<(), sqlx::Error> {
        let now: DateTime<Utc> = self.clock.system_time().into();
        let now_str = now.to_rfc3339();

        let pending =
            NotificationOutboxRepo::list_pending(&self.pool, &now_str, BATCH_SIZE).await?;

        for entry in &pending {
            let notifier_id = match NotifierId::new(&entry.notifier_id) {
                Ok(id) => id,
                Err(_) => {
                    tracing::warn!(notifier_id = %entry.notifier_id, "invalid notifier id");
                    continue;
                }
            };

            let notifier = {
                let notifiers = self.notifiers.read().unwrap();
                notifiers.get(&notifier_id).cloned()
            };

            let notifier = match notifier {
                Some(n) => n,
                None => {
                    tracing::warn!(
                        notifier_id = %entry.notifier_id,
                        "notifier not found, marking as failed"
                    );
                    if entry.attempt_count + 1 >= MAX_RETRY_ATTEMPTS {
                        NotificationOutboxRepo::mark_failed(
                            &self.pool,
                            entry.internal_id,
                            "notifier not configured",
                        )
                        .await?;
                    } else {
                        let backoff = compute_backoff(entry.attempt_count);
                        let next = now + chrono::Duration::from_std(backoff).unwrap_or_default();
                        NotificationOutboxRepo::mark_retry(
                            &self.pool,
                            entry.internal_id,
                            &next.to_rfc3339(),
                            "notifier not configured",
                        )
                        .await?;
                    }
                    continue;
                }
            };

            let payload = match self.build_payload(entry.alert_event_internal_id).await {
                Ok(p) => p,
                Err(e) => {
                    tracing::error!(error = %e, "failed to build notification payload");
                    if entry.attempt_count + 1 >= MAX_RETRY_ATTEMPTS {
                        NotificationOutboxRepo::mark_failed(
                            &self.pool,
                            entry.internal_id,
                            &e.to_string(),
                        )
                        .await?;
                    } else {
                        let backoff = compute_backoff(entry.attempt_count);
                        let next = now + chrono::Duration::from_std(backoff).unwrap_or_default();
                        NotificationOutboxRepo::mark_retry(
                            &self.pool,
                            entry.internal_id,
                            &next.to_rfc3339(),
                            &e.to_string(),
                        )
                        .await?;
                    }
                    continue;
                }
            };

            let notifier_type = notifier.kind().to_owned();
            match notifier.send(payload).await {
                Ok(()) => {
                    NotificationOutboxRepo::mark_delivered(&self.pool, entry.internal_id).await?;
                    metrics::counter!("kemuri_notification_attempts_total",
                        "notifier_type" => notifier_type.clone(),
                        "result" => "success")
                    .increment(1);
                    metrics::gauge!("kemuri_notification_outbox_pending").decrement(1.0);
                }
                Err(e) => {
                    tracing::warn!(
                        notifier_id = %entry.notifier_id,
                        error = %e,
                        "notification delivery failed"
                    );
                    metrics::counter!("kemuri_notification_attempts_total",
                        "notifier_type" => notifier_type,
                        "result" => "failure")
                    .increment(1);

                    if entry.attempt_count + 1 >= MAX_RETRY_ATTEMPTS {
                        NotificationOutboxRepo::mark_failed(
                            &self.pool,
                            entry.internal_id,
                            &e.to_string(),
                        )
                        .await?;
                    } else {
                        let backoff = compute_backoff(entry.attempt_count);
                        let next = now + chrono::Duration::from_std(backoff).unwrap_or_default();
                        NotificationOutboxRepo::mark_retry(
                            &self.pool,
                            entry.internal_id,
                            &next.to_rfc3339(),
                            &e.to_string(),
                        )
                        .await?;
                    }
                }
            }
        }

        Ok(())
    }

    async fn build_payload(
        &self,
        event_internal_id: i64,
    ) -> Result<NotificationPayload, BuildPayloadError> {
        let event = AlertEventRepo::get_by_internal_id(&self.pool, event_internal_id)
            .await
            .map_err(|e| BuildPayloadError(e.to_string()))?
            .ok_or_else(|| BuildPayloadError("event not found".to_owned()))?;

        let alert_state = AlertStateRepo::get(
            &self.pool,
            &event.rule_id,
            event.check_internal_id,
            event.observer_internal_id,
        )
        .await
        .map_err(|e| BuildPayloadError(e.to_string()))?;

        let check = CheckRepo::get_by_internal_id(&self.pool, event.check_internal_id)
            .await
            .map_err(|e| BuildPayloadError(e.to_string()))?
            .ok_or_else(|| BuildPayloadError("check not found".to_owned()))?;

        let target = TargetRepo::get_by_internal_id(&self.pool, check.target_internal_id)
            .await
            .map_err(|e| BuildPayloadError(e.to_string()))?
            .ok_or_else(|| BuildPayloadError("target not found".to_owned()))?;

        let event_type = match event.event_type.as_str() {
            "firing" => AlertEventKind::Firing,
            _ => AlertEventKind::Resolved,
        };

        let state_start_time = alert_state
            .as_ref()
            .and_then(|s| DateTime::parse_from_rfc3339(&s.state_entered_at).ok())
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_default();

        let event_time = DateTime::parse_from_rfc3339(&event.occurred_at)
            .map(|dt| dt.with_timezone(&Utc))
            .unwrap_or_default();

        let labels: HashMap<String, String> =
            serde_json::from_str(&target.labels).unwrap_or_default();

        let summary = match event_type {
            AlertEventKind::Firing => format!(
                "Alert {} is firing for {}/{}: current value {} exceeds threshold {}",
                event.rule_id,
                target.name,
                check.check_id,
                event.metric_value.unwrap_or(0.0),
                event.threshold_value.unwrap_or(0.0)
            ),
            AlertEventKind::Resolved => format!(
                "Alert {} resolved for {}/{}: current value {} is below threshold {}",
                event.rule_id,
                target.name,
                check.check_id,
                event.metric_value.unwrap_or(0.0),
                event.threshold_value.unwrap_or(0.0)
            ),
        };

        let kemuri_url = self.public_url.as_ref().map(|base| {
            format!(
                "{}/targets/{}/checks/{}",
                base.trim_end_matches('/'),
                target.target_id,
                check.check_id
            )
        });

        Ok(NotificationPayload {
            event_id: event.internal_id.to_string(),
            event_type,
            rule_id: kemuri_core::RuleId::new(&event.rule_id)
                .unwrap_or_else(|_| kemuri_core::RuleId::new("unknown").unwrap()),
            target_id: kemuri_core::TargetId::new(&target.target_id)
                .unwrap_or_else(|_| kemuri_core::TargetId::new("unknown").unwrap()),
            target_name: target.name,
            check_id: kemuri_core::CheckId::new(&check.check_id)
                .unwrap_or_else(|_| kemuri_core::CheckId::new("unknown").unwrap()),
            observer_id: kemuri_core::ObserverId::new("local")
                .unwrap_or_else(|_| kemuri_core::ObserverId::new("obs").unwrap()),
            probe_type: check
                .probe_type
                .parse()
                .unwrap_or(kemuri_core::ProbeKind::Icmp),
            current_value: event.metric_value.unwrap_or(0.0),
            threshold: event.threshold_value.unwrap_or(0.0),
            state_start_time,
            event_time,
            kemuri_url,
            labels,
            summary,
        })
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct BuildPayloadError(String);
