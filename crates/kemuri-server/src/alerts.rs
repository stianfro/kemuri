use std::sync::Arc;

use chrono::{DateTime, Utc};
use kemuri_config::{AlertRuleConfig, KemuriConfig};
use kemuri_core::{AlertEventKind, AlertState, CheckId, Clock, TargetId, parse_duration};
use kemuri_storage::{
    AlertEventRepo, AlertStateRepo, CheckRepo, InsertAlertEvent, InsertNotificationOutbox,
    NotificationOutboxRepo, RoundRepo, TargetRepo, UpsertAlertState,
};
use sqlx::SqlitePool;
use tokio::sync::mpsc;

use crate::events::SystemEvent;

const MAX_RETRY_ATTEMPTS: i64 = 10;
const RETRY_BASE_SECS: u64 = 30;
const RETRY_MAX_SECS: u64 = 3600;

pub struct AlertEvaluator {
    pool: Arc<SqlitePool>,
    config: Arc<std::sync::RwLock<Arc<KemuriConfig>>>,
    clock: Arc<dyn Clock>,
    observer_internal_id: i64,
    event_tx: Option<tokio::sync::broadcast::Sender<SystemEvent>>,
}

#[derive(Debug, Clone)]
pub struct AlertNotification {
    pub target_id: TargetId,
    pub check_id: CheckId,
    pub scheduled_at: DateTime<Utc>,
}

impl AlertEvaluator {
    pub fn new(
        pool: Arc<SqlitePool>,
        config: Arc<std::sync::RwLock<Arc<KemuriConfig>>>,
        clock: Arc<dyn Clock>,
        observer_internal_id: i64,
    ) -> Self {
        Self {
            pool,
            config,
            clock,
            observer_internal_id,
            event_tx: None,
        }
    }

    pub fn with_event_channel(
        mut self,
        event_tx: tokio::sync::broadcast::Sender<SystemEvent>,
    ) -> Self {
        self.event_tx = Some(event_tx);
        self
    }

    pub async fn run(&self, mut rx: mpsc::Receiver<AlertNotification>) {
        while let Some(notif) = rx.recv().await {
            if let Err(e) = self.evaluate_round(&notif).await {
                tracing::error!(
                    target_id = %notif.target_id,
                    check_id = %notif.check_id,
                    error = %e,
                    "alert evaluation failed"
                );
                metrics::counter!("kemuri_alert_eval_errors").increment(1);
            }
        }
        tracing::info!("alert evaluator shutting down");
    }

    pub async fn run_no_data_check(&self) {
        if let Err(e) = self.evaluate_no_data().await {
            tracing::error!(error = %e, "no-data alert evaluation failed");
        }
    }

    async fn evaluate_round(&self, notif: &AlertNotification) -> Result<(), sqlx::Error> {
        let target_row = TargetRepo::get_by_target_id(&self.pool, notif.target_id.as_str())
            .await?
            .unwrap_or_else(|| panic!("target not found: {}", notif.target_id));

        let check_row = CheckRepo::get(&self.pool, target_row.internal_id, notif.check_id.as_str())
            .await?
            .unwrap_or_else(|| panic!("check not found: {}", notif.check_id));

        if !check_row.active {
            return Ok(());
        }

        let config = self.config.read().unwrap().clone();
        let applicable_rules: Vec<&AlertRuleConfig> = config
            .rules
            .iter()
            .filter(|r| {
                let profile = config.profiles.iter().find(|p| p.id() == &r.profile);
                profile.is_some_and(|p| p.kind().to_string() == check_row.probe_type)
            })
            .collect();

        for rule in applicable_rules {
            self.evaluate_rule(rule, check_row.internal_id).await?;
        }

        Ok(())
    }

    async fn evaluate_no_data(&self) -> Result<(), sqlx::Error> {
        let config = self.config.read().unwrap().clone();
        let no_data_rules: Vec<&AlertRuleConfig> = config
            .rules
            .iter()
            .filter(|r| r.metric == "no_data")
            .collect();

        if no_data_rules.is_empty() {
            return Ok(());
        }

        let now: DateTime<Utc> = self.clock.system_time().into();
        let now_str = now.to_rfc3339();

        let active_checks = CheckRepo::list_active_with_target(&self.pool).await?;

        for rule in &no_data_rules {
            let _profile = match config.profiles.iter().find(|p| p.id() == &rule.profile) {
                Some(p) => p,
                None => continue,
            };

            for (check_internal_id, _probe_type) in &active_checks {
                let period = rule
                    .no_data_period
                    .as_deref()
                    .or(Some(rule.window.as_str()))
                    .and_then(|s| parse_duration(s).ok())
                    .unwrap_or(std::time::Duration::from_secs(300));
                let cutoff = now - chrono::Duration::from_std(period).unwrap_or_default();
                let cutoff_str = cutoff.to_rfc3339();

                let has_rounds = RoundRepo::has_rounds_since(
                    &self.pool,
                    *check_internal_id,
                    self.observer_internal_id,
                    &cutoff_str,
                )
                .await?;

                if !has_rounds {
                    self.evaluate_no_data_rule(rule, *check_internal_id, &now_str)
                        .await?;
                }
            }
        }

        Ok(())
    }

    async fn evaluate_no_data_rule(
        &self,
        rule: &AlertRuleConfig,
        check_internal_id: i64,
        now_str: &str,
    ) -> Result<(), sqlx::Error> {
        let existing = AlertStateRepo::get(
            &self.pool,
            rule.id.as_str(),
            check_internal_id,
            self.observer_internal_id,
        )
        .await?;

        let current_state = existing
            .as_ref()
            .map(|r| parse_alert_state(&r.state))
            .unwrap_or(AlertState::Normal);

        let metric_value = 1.0;
        let threshold = 1.0;

        match current_state {
            AlertState::Normal | AlertState::PendingClear => {
                let prev_state_str = current_state.to_string();
                let new_state = AlertState::PendingFire;
                let first_condition = existing
                    .as_ref()
                    .and_then(|r| r.first_condition_true_at.clone())
                    .unwrap_or_else(|| now_str.to_owned());

                let should_fire = should_transition_to_firing(rule, &first_condition, now_str);

                let (final_state, event_kind) = if should_fire {
                    (AlertState::Firing, AlertEventKind::Firing)
                } else {
                    (new_state, AlertEventKind::Firing)
                };

                let upsert = UpsertAlertState {
                    rule_id: rule.id.to_string(),
                    check_internal_id,
                    observer_internal_id: self.observer_internal_id,
                    state: final_state.to_string(),
                    state_entered_at: now_str.to_owned(),
                    first_condition_true_at: Some(first_condition.clone()),
                    last_evaluated_at: Some(now_str.to_owned()),
                    last_notification_at: existing
                        .as_ref()
                        .and_then(|r| r.last_notification_at.clone()),
                    fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                    last_metric_value: Some(metric_value),
                };

                AlertStateRepo::upsert(&self.pool, &upsert).await?;

                if should_fire {
                    let event = InsertAlertEvent {
                        rule_id: rule.id.to_string(),
                        check_internal_id,
                        observer_internal_id: self.observer_internal_id,
                        event_type: event_kind.to_string(),
                        from_state: prev_state_str,
                        to_state: final_state.to_string(),
                        metric_value: Some(metric_value),
                        threshold_value: Some(threshold),
                        occurred_at: now_str.to_owned(),
                    };
                    let event_id = AlertEventRepo::insert(&self.pool, &event).await?;
                    self.insert_outbox_entry(event_id, rule).await?;
                    self.publish_alert_event(&event_kind, rule, check_internal_id);
                }
            }
            AlertState::Firing => {
                let should_repeat = should_repeat_notification(
                    rule,
                    existing
                        .as_ref()
                        .and_then(|r| r.last_notification_at.as_deref()),
                    now_str,
                );

                let upsert = UpsertAlertState {
                    rule_id: rule.id.to_string(),
                    check_internal_id,
                    observer_internal_id: self.observer_internal_id,
                    state: AlertState::Firing.to_string(),
                    state_entered_at: existing
                        .as_ref()
                        .map(|r| r.state_entered_at.clone())
                        .unwrap_or_else(|| now_str.to_owned()),
                    first_condition_true_at: existing
                        .as_ref()
                        .and_then(|r| r.first_condition_true_at.clone()),
                    last_evaluated_at: Some(now_str.to_owned()),
                    last_notification_at: if should_repeat {
                        Some(now_str.to_owned())
                    } else {
                        existing.and_then(|r| r.last_notification_at)
                    },
                    fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                    last_metric_value: Some(metric_value),
                };

                AlertStateRepo::upsert(&self.pool, &upsert).await?;

                if should_repeat {
                    let event = InsertAlertEvent {
                        rule_id: rule.id.to_string(),
                        check_internal_id,
                        observer_internal_id: self.observer_internal_id,
                        event_type: AlertEventKind::Firing.to_string(),
                        from_state: AlertState::Firing.to_string(),
                        to_state: AlertState::Firing.to_string(),
                        metric_value: Some(metric_value),
                        threshold_value: Some(threshold),
                        occurred_at: now_str.to_owned(),
                    };
                    let event_id = AlertEventRepo::insert(&self.pool, &event).await?;
                    self.insert_outbox_entry(event_id, rule).await?;
                }
            }
            AlertState::PendingFire => {}
        }

        Ok(())
    }

    async fn evaluate_rule(
        &self,
        rule: &AlertRuleConfig,
        check_internal_id: i64,
    ) -> Result<(), sqlx::Error> {
        let now: DateTime<Utc> = self.clock.system_time().into();
        let now_str = now.to_rfc3339();

        let window_duration =
            parse_duration(&rule.window).unwrap_or(std::time::Duration::from_secs(300));
        let window_ago = now - chrono::Duration::from_std(window_duration).unwrap_or_default();
        let window_ago_str = window_ago.to_rfc3339();

        let rounds = RoundRepo::query_by_check_range_with_observer(
            &self.pool,
            check_internal_id,
            self.observer_internal_id,
            &window_ago_str,
            &now_str,
        )
        .await?;

        let minimum_rounds = rule.minimum_rounds.unwrap_or(1) as usize;
        if rounds.len() < minimum_rounds {
            return Ok(());
        }

        let metric_result = compute_metric(&rule.metric, &rounds);
        let metric_value = match metric_result {
            MetricResult::InsufficientData => return Ok(()),
            MetricResult::Value(v) => v,
        };

        let threshold = parse_threshold(&rule.threshold, &rule.metric);
        let fire_condition = evaluate_condition(metric_value, threshold, &rule.operator);

        let existing = AlertStateRepo::get(
            &self.pool,
            rule.id.as_str(),
            check_internal_id,
            self.observer_internal_id,
        )
        .await?;

        let current_state = existing
            .as_ref()
            .map(|r| parse_alert_state(&r.state))
            .unwrap_or(AlertState::Normal);

        self.update_state_machine(
            rule,
            check_internal_id,
            &existing,
            current_state,
            fire_condition,
            metric_value,
            threshold,
            &now_str,
        )
        .await?;

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_state_machine(
        &self,
        rule: &AlertRuleConfig,
        check_internal_id: i64,
        existing: &Option<kemuri_storage::AlertStateRow>,
        current_state: AlertState,
        fire_condition: bool,
        metric_value: f64,
        threshold: f64,
        now_str: &str,
    ) -> Result<(), sqlx::Error> {
        let clear_threshold = rule
            .clear_threshold
            .as_deref()
            .map(|ct| parse_threshold(ct, &rule.metric))
            .unwrap_or_else(|| invert_threshold(threshold, &rule.operator));
        let clear_operator = rule
            .clear_operator
            .as_deref()
            .unwrap_or_else(|| invert_operator(&rule.operator));
        let clear_condition = evaluate_condition(metric_value, clear_threshold, clear_operator);

        match current_state {
            AlertState::Normal => {
                if fire_condition {
                    let first_condition = existing
                        .as_ref()
                        .and_then(|r| r.first_condition_true_at.clone())
                        .unwrap_or_else(|| now_str.to_owned());

                    let should_fire = should_transition_to_firing(rule, &first_condition, now_str);

                    let (new_state, event_kind_opt) = if should_fire {
                        (AlertState::Firing, Some(AlertEventKind::Firing))
                    } else {
                        (AlertState::PendingFire, None)
                    };

                    let upsert = UpsertAlertState {
                        rule_id: rule.id.to_string(),
                        check_internal_id,
                        observer_internal_id: self.observer_internal_id,
                        state: new_state.to_string(),
                        state_entered_at: now_str.to_owned(),
                        first_condition_true_at: Some(first_condition),
                        last_evaluated_at: Some(now_str.to_owned()),
                        last_notification_at: None,
                        fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                        last_metric_value: Some(metric_value),
                    };

                    AlertStateRepo::upsert(&self.pool, &upsert).await?;

                    if let Some(event_kind) = event_kind_opt {
                        let event = InsertAlertEvent {
                            rule_id: rule.id.to_string(),
                            check_internal_id,
                            observer_internal_id: self.observer_internal_id,
                            event_type: event_kind.to_string(),
                            from_state: AlertState::Normal.to_string(),
                            to_state: new_state.to_string(),
                            metric_value: Some(metric_value),
                            threshold_value: Some(threshold),
                            occurred_at: now_str.to_owned(),
                        };
                        let event_id = AlertEventRepo::insert(&self.pool, &event).await?;
                        self.insert_outbox_entry(event_id, rule).await?;
                        self.publish_alert_event(&event_kind, rule, check_internal_id);
                    }
                } else {
                    let upsert = UpsertAlertState {
                        rule_id: rule.id.to_string(),
                        check_internal_id,
                        observer_internal_id: self.observer_internal_id,
                        state: AlertState::Normal.to_string(),
                        state_entered_at: existing
                            .as_ref()
                            .map(|r| r.state_entered_at.clone())
                            .unwrap_or_else(|| now_str.to_owned()),
                        first_condition_true_at: None,
                        last_evaluated_at: Some(now_str.to_owned()),
                        last_notification_at: None,
                        fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                        last_metric_value: Some(metric_value),
                    };
                    AlertStateRepo::upsert(&self.pool, &upsert).await?;
                }
            }
            AlertState::PendingFire => {
                if !fire_condition {
                    let upsert = UpsertAlertState {
                        rule_id: rule.id.to_string(),
                        check_internal_id,
                        observer_internal_id: self.observer_internal_id,
                        state: AlertState::Normal.to_string(),
                        state_entered_at: now_str.to_owned(),
                        first_condition_true_at: None,
                        last_evaluated_at: Some(now_str.to_owned()),
                        last_notification_at: None,
                        fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                        last_metric_value: Some(metric_value),
                    };
                    AlertStateRepo::upsert(&self.pool, &upsert).await?;
                } else {
                    let first_condition = existing
                        .as_ref()
                        .and_then(|r| r.first_condition_true_at.clone())
                        .unwrap_or_else(|| now_str.to_owned());

                    let should_fire = should_transition_to_firing(rule, &first_condition, now_str);

                    if should_fire {
                        let upsert = UpsertAlertState {
                            rule_id: rule.id.to_string(),
                            check_internal_id,
                            observer_internal_id: self.observer_internal_id,
                            state: AlertState::Firing.to_string(),
                            state_entered_at: now_str.to_owned(),
                            first_condition_true_at: Some(first_condition),
                            last_evaluated_at: Some(now_str.to_owned()),
                            last_notification_at: Some(now_str.to_owned()),
                            fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                            last_metric_value: Some(metric_value),
                        };
                        AlertStateRepo::upsert(&self.pool, &upsert).await?;

                        let event = InsertAlertEvent {
                            rule_id: rule.id.to_string(),
                            check_internal_id,
                            observer_internal_id: self.observer_internal_id,
                            event_type: AlertEventKind::Firing.to_string(),
                            from_state: AlertState::PendingFire.to_string(),
                            to_state: AlertState::Firing.to_string(),
                            metric_value: Some(metric_value),
                            threshold_value: Some(threshold),
                            occurred_at: now_str.to_owned(),
                        };
                        let event_id = AlertEventRepo::insert(&self.pool, &event).await?;
                        self.insert_outbox_entry(event_id, rule).await?;
                        self.publish_alert_event(&AlertEventKind::Firing, rule, check_internal_id);
                    } else {
                        let upsert = UpsertAlertState {
                            rule_id: rule.id.to_string(),
                            check_internal_id,
                            observer_internal_id: self.observer_internal_id,
                            state: AlertState::PendingFire.to_string(),
                            state_entered_at: existing
                                .as_ref()
                                .map(|r| r.state_entered_at.clone())
                                .unwrap_or_else(|| now_str.to_owned()),
                            first_condition_true_at: Some(first_condition),
                            last_evaluated_at: Some(now_str.to_owned()),
                            last_notification_at: None,
                            fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                            last_metric_value: Some(metric_value),
                        };
                        AlertStateRepo::upsert(&self.pool, &upsert).await?;
                    }
                }
            }
            AlertState::Firing => {
                if clear_condition {
                    let should_clear = should_transition_to_clear(rule, existing, now_str);

                    let (new_state, event_kind_opt) = if should_clear {
                        (AlertState::Normal, Some(AlertEventKind::Resolved))
                    } else {
                        (AlertState::PendingClear, None)
                    };

                    let first_condition = existing
                        .as_ref()
                        .and_then(|r| r.first_condition_true_at.clone());

                    let upsert = UpsertAlertState {
                        rule_id: rule.id.to_string(),
                        check_internal_id,
                        observer_internal_id: self.observer_internal_id,
                        state: new_state.to_string(),
                        state_entered_at: now_str.to_owned(),
                        first_condition_true_at: if should_clear { None } else { first_condition },
                        last_evaluated_at: Some(now_str.to_owned()),
                        last_notification_at: if should_clear {
                            Some(now_str.to_owned())
                        } else {
                            existing
                                .as_ref()
                                .and_then(|r| r.last_notification_at.clone())
                        },
                        fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                        last_metric_value: Some(metric_value),
                    };
                    AlertStateRepo::upsert(&self.pool, &upsert).await?;

                    if let Some(event_kind) = event_kind_opt {
                        let event = InsertAlertEvent {
                            rule_id: rule.id.to_string(),
                            check_internal_id,
                            observer_internal_id: self.observer_internal_id,
                            event_type: event_kind.to_string(),
                            from_state: AlertState::Firing.to_string(),
                            to_state: new_state.to_string(),
                            metric_value: Some(metric_value),
                            threshold_value: Some(clear_threshold),
                            occurred_at: now_str.to_owned(),
                        };
                        let event_id = AlertEventRepo::insert(&self.pool, &event).await?;
                        self.insert_outbox_entry(event_id, rule).await?;
                        self.publish_alert_event(&event_kind, rule, check_internal_id);
                    }
                } else {
                    let should_repeat = should_repeat_notification(
                        rule,
                        existing
                            .as_ref()
                            .and_then(|r| r.last_notification_at.as_deref()),
                        now_str,
                    );

                    let upsert = UpsertAlertState {
                        rule_id: rule.id.to_string(),
                        check_internal_id,
                        observer_internal_id: self.observer_internal_id,
                        state: AlertState::Firing.to_string(),
                        state_entered_at: existing
                            .as_ref()
                            .map(|r| r.state_entered_at.clone())
                            .unwrap_or_else(|| now_str.to_owned()),
                        first_condition_true_at: existing
                            .as_ref()
                            .and_then(|r| r.first_condition_true_at.clone()),
                        last_evaluated_at: Some(now_str.to_owned()),
                        last_notification_at: if should_repeat {
                            Some(now_str.to_owned())
                        } else {
                            existing
                                .as_ref()
                                .and_then(|r| r.last_notification_at.clone())
                        },
                        fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                        last_metric_value: Some(metric_value),
                    };
                    AlertStateRepo::upsert(&self.pool, &upsert).await?;

                    if should_repeat {
                        let event = InsertAlertEvent {
                            rule_id: rule.id.to_string(),
                            check_internal_id,
                            observer_internal_id: self.observer_internal_id,
                            event_type: AlertEventKind::Firing.to_string(),
                            from_state: AlertState::Firing.to_string(),
                            to_state: AlertState::Firing.to_string(),
                            metric_value: Some(metric_value),
                            threshold_value: Some(threshold),
                            occurred_at: now_str.to_owned(),
                        };
                        let event_id = AlertEventRepo::insert(&self.pool, &event).await?;
                        self.insert_outbox_entry(event_id, rule).await?;
                    }
                }
            }
            AlertState::PendingClear => {
                if !clear_condition {
                    let upsert = UpsertAlertState {
                        rule_id: rule.id.to_string(),
                        check_internal_id,
                        observer_internal_id: self.observer_internal_id,
                        state: AlertState::Firing.to_string(),
                        state_entered_at: existing
                            .as_ref()
                            .map(|r| r.state_entered_at.clone())
                            .unwrap_or_else(|| now_str.to_owned()),
                        first_condition_true_at: existing
                            .as_ref()
                            .and_then(|r| r.first_condition_true_at.clone()),
                        last_evaluated_at: Some(now_str.to_owned()),
                        last_notification_at: existing
                            .as_ref()
                            .and_then(|r| r.last_notification_at.clone()),
                        fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                        last_metric_value: Some(metric_value),
                    };
                    AlertStateRepo::upsert(&self.pool, &upsert).await?;
                } else {
                    let should_clear = should_transition_to_clear(rule, existing, now_str);

                    if should_clear {
                        let upsert = UpsertAlertState {
                            rule_id: rule.id.to_string(),
                            check_internal_id,
                            observer_internal_id: self.observer_internal_id,
                            state: AlertState::Normal.to_string(),
                            state_entered_at: now_str.to_owned(),
                            first_condition_true_at: None,
                            last_evaluated_at: Some(now_str.to_owned()),
                            last_notification_at: Some(now_str.to_owned()),
                            fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                            last_metric_value: Some(metric_value),
                        };
                        AlertStateRepo::upsert(&self.pool, &upsert).await?;

                        let event = InsertAlertEvent {
                            rule_id: rule.id.to_string(),
                            check_internal_id,
                            observer_internal_id: self.observer_internal_id,
                            event_type: AlertEventKind::Resolved.to_string(),
                            from_state: AlertState::PendingClear.to_string(),
                            to_state: AlertState::Normal.to_string(),
                            metric_value: Some(metric_value),
                            threshold_value: Some(clear_threshold),
                            occurred_at: now_str.to_owned(),
                        };
                        let event_id = AlertEventRepo::insert(&self.pool, &event).await?;
                        self.insert_outbox_entry(event_id, rule).await?;
                        self.publish_alert_event(
                            &AlertEventKind::Resolved,
                            rule,
                            check_internal_id,
                        );
                    } else {
                        let upsert = UpsertAlertState {
                            rule_id: rule.id.to_string(),
                            check_internal_id,
                            observer_internal_id: self.observer_internal_id,
                            state: AlertState::PendingClear.to_string(),
                            state_entered_at: existing
                                .as_ref()
                                .map(|r| r.state_entered_at.clone())
                                .unwrap_or_else(|| now_str.to_owned()),
                            first_condition_true_at: existing
                                .as_ref()
                                .and_then(|r| r.first_condition_true_at.clone()),
                            last_evaluated_at: Some(now_str.to_owned()),
                            last_notification_at: existing
                                .as_ref()
                                .and_then(|r| r.last_notification_at.clone()),
                            fingerprint: Some(compute_fingerprint(rule, check_internal_id)),
                            last_metric_value: Some(metric_value),
                        };
                        AlertStateRepo::upsert(&self.pool, &upsert).await?;
                    }
                }
            }
        }

        Ok(())
    }

    fn publish_alert_event(
        &self,
        event_kind: &AlertEventKind,
        rule: &AlertRuleConfig,
        check_internal_id: i64,
    ) {
        if let Some(ref event_tx) = self.event_tx {
            let pool = self.pool.clone();
            let rule_id = rule.id.to_string();
            let kind = match event_kind {
                AlertEventKind::Firing => "firing",
                AlertEventKind::Resolved => "resolved",
            };
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                let check = match CheckRepo::get_by_internal_id(&pool, check_internal_id).await {
                    Ok(Some(c)) => c,
                    _ => return,
                };
                let target =
                    match TargetRepo::get_by_internal_id(&pool, check.target_internal_id).await {
                        Ok(Some(t)) => t,
                        _ => return,
                    };
                let tid = target.target_id;
                let _ = event_tx.send(match kind {
                    "firing" => SystemEvent::alert_firing(&rule_id, &tid, ""),
                    "resolved" => SystemEvent::alert_resolved(&rule_id, &tid, ""),
                    _ => return,
                });
            });
        }

        metrics::counter!("kemuri_alert_transitions_total",
            "transition" => event_kind.to_string())
        .increment(1);
        metrics::gauge!("kemuri_alert_instances",
            "state" => event_kind.to_string())
        .increment(1.0);
    }

    async fn insert_outbox_entry(
        &self,
        event_id: i64,
        rule: &AlertRuleConfig,
    ) -> Result<(), sqlx::Error> {
        let now: DateTime<Utc> = self.clock.system_time().into();
        let entry = InsertNotificationOutbox {
            alert_event_internal_id: event_id,
            notifier_id: rule.notifier.to_string(),
            status: "pending".to_owned(),
            next_attempt_at: now.to_rfc3339(),
        };
        NotificationOutboxRepo::insert(&self.pool, &entry).await?;
        metrics::gauge!("kemuri_notification_outbox_pending").increment(1.0);
        Ok(())
    }
}

enum MetricResult {
    InsufficientData,
    Value(f64),
}

fn compute_metric(metric: &str, rounds: &[kemuri_storage::RoundRow]) -> MetricResult {
    match metric {
        "measurement_loss_ratio" => {
            let total_attempted: i64 = rounds.iter().map(|r| r.attempted_samples as i64).sum();
            if total_attempted == 0 {
                return MetricResult::InsufficientData;
            }
            let total_lost: i64 = rounds
                .iter()
                .map(|r| r.measurement_loss_samples as i64)
                .sum();
            MetricResult::Value(total_lost as f64 / total_attempted as f64)
        }
        "health_failure_ratio" => {
            let total_attempted: i64 = rounds.iter().map(|r| r.attempted_samples as i64).sum();
            if total_attempted == 0 {
                return MetricResult::InsufficientData;
            }
            let total_unhealthy: i64 = rounds.iter().map(|r| r.unhealthy_samples as i64).sum();
            MetricResult::Value(total_unhealthy as f64 / total_attempted as f64)
        }
        "healthy_sample_ratio" => {
            let total_attempted: i64 = rounds.iter().map(|r| r.attempted_samples as i64).sum();
            if total_attempted == 0 {
                return MetricResult::InsufficientData;
            }
            let total_healthy: i64 = rounds.iter().map(|r| r.healthy_samples as i64).sum();
            MetricResult::Value(total_healthy as f64 / total_attempted as f64)
        }
        "p50_latency" | "p95_latency" | "p99_latency" => {
            let p = match metric {
                "p50_latency" => 0.5,
                "p95_latency" => 0.95,
                "p99_latency" => 0.99,
                _ => 0.5,
            };
            let mut histogram = kemuri_core::Histogram::new();
            let mut has_data = false;
            for round in rounds {
                if let Some(ref blob) = round.sample_blob
                    && let Ok(records) = kemuri_core::decode_samples(blob)
                {
                    for record in &records {
                        if let Some(lat_ns) = record.latency_ns {
                            histogram.record(lat_ns);
                            has_data = true;
                        }
                    }
                }
            }
            if !has_data {
                return MetricResult::InsufficientData;
            }
            match histogram.quantile(p) {
                Some(ns) => MetricResult::Value(ns as f64 / 1_000_000.0),
                None => MetricResult::InsufficientData,
            }
        }
        "consecutive_total_loss_rounds" => {
            let sorted: Vec<&kemuri_storage::RoundRow> = {
                let mut r: Vec<&kemuri_storage::RoundRow> = rounds.iter().collect();
                r.sort_by(|a, b| a.scheduled_at.cmp(&b.scheduled_at));
                r
            };
            let mut count: f64 = 0.0;
            for round in sorted.iter().rev() {
                if round.attempted_samples > 0
                    && round.measurement_loss_samples == round.attempted_samples
                {
                    count += 1.0;
                } else {
                    break;
                }
            }
            MetricResult::Value(count)
        }
        "consecutive_unhealthy_rounds" => {
            let sorted: Vec<&kemuri_storage::RoundRow> = {
                let mut r: Vec<&kemuri_storage::RoundRow> = rounds.iter().collect();
                r.sort_by(|a, b| a.scheduled_at.cmp(&b.scheduled_at));
                r
            };
            let mut count: f64 = 0.0;
            for round in sorted.iter().rev() {
                if round.unhealthy_samples > 0 {
                    count += 1.0;
                } else {
                    break;
                }
            }
            MetricResult::Value(count)
        }
        "no_data" => MetricResult::Value(0.0),
        _ => MetricResult::InsufficientData,
    }
}

fn parse_threshold(threshold: &str, metric: &str) -> f64 {
    if threshold.ends_with('%') {
        let trimmed = threshold.trim_end_matches('%');
        trimmed.parse::<f64>().unwrap_or(0.0) / 100.0
    } else if matches!(metric, "p50_latency" | "p95_latency" | "p99_latency") {
        parse_duration(threshold)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or_else(|_| threshold.parse().unwrap_or(0.0))
    } else {
        threshold.parse().unwrap_or(0.0)
    }
}

fn evaluate_condition(value: f64, threshold: f64, operator: &str) -> bool {
    match operator {
        "gt" => value > threshold,
        "gte" => value >= threshold,
        "lt" => value < threshold,
        "lte" => value <= threshold,
        _ => false,
    }
}

fn parse_alert_state(s: &str) -> AlertState {
    match s {
        "pending_fire" => AlertState::PendingFire,
        "firing" => AlertState::Firing,
        "pending_clear" => AlertState::PendingClear,
        _ => AlertState::Normal,
    }
}

fn should_transition_to_firing(
    rule: &AlertRuleConfig,
    first_condition_true_at: &str,
    now_str: &str,
) -> bool {
    if let Some(ref duration_str) = rule.duration
        && let Ok(duration) = parse_duration(duration_str)
    {
        let first = DateTime::parse_from_rfc3339(first_condition_true_at);
        let now = DateTime::parse_from_rfc3339(now_str);
        match (first, now) {
            (Ok(f), Ok(n)) => {
                let elapsed = n - f;
                let required = chrono::Duration::from_std(duration).unwrap_or_default();
                return elapsed >= required;
            }
            _ => return true,
        }
    }
    true
}

fn should_transition_to_clear(
    rule: &AlertRuleConfig,
    existing: &Option<kemuri_storage::AlertStateRow>,
    now_str: &str,
) -> bool {
    if let Some(ref duration_str) = rule.duration
        && let Ok(duration) = parse_duration(duration_str)
    {
        let state_entered = existing
            .as_ref()
            .map(|r| r.state_entered_at.clone())
            .unwrap_or_default();
        let entered = DateTime::parse_from_rfc3339(&state_entered);
        let now = DateTime::parse_from_rfc3339(now_str);
        match (entered, now) {
            (Ok(e), Ok(n)) => {
                let elapsed = n - e;
                let required = chrono::Duration::from_std(duration).unwrap_or_default();
                return elapsed >= required;
            }
            _ => return true,
        }
    }
    true
}

fn should_repeat_notification(
    rule: &AlertRuleConfig,
    last_notification_at: Option<&str>,
    now_str: &str,
) -> bool {
    let repeat_every = match rule.repeat_every.as_deref() {
        Some(s) => match parse_duration(s) {
            Ok(d) => d,
            Err(_) => return false,
        },
        None => return false,
    };

    let last = match last_notification_at {
        Some(s) => match DateTime::parse_from_rfc3339(s) {
            Ok(dt) => dt,
            Err(_) => return true,
        },
        None => return true,
    };

    let now = match DateTime::parse_from_rfc3339(now_str) {
        Ok(dt) => dt,
        Err(_) => return false,
    };

    let elapsed = now - last;
    let required = chrono::Duration::from_std(repeat_every).unwrap_or_default();
    elapsed >= required
}

fn invert_threshold(threshold: f64, operator: &str) -> f64 {
    match operator {
        "gt" | "gte" => threshold * 0.9,
        "lt" | "lte" => threshold * 1.1,
        _ => threshold,
    }
}

fn invert_operator(operator: &str) -> &'static str {
    match operator {
        "gt" => "lte",
        "gte" => "lt",
        "lt" => "gte",
        "lte" => "gt",
        _ => "lt",
    }
}

fn compute_fingerprint(rule: &AlertRuleConfig, check_internal_id: i64) -> String {
    format!("{}:{}", rule.id, check_internal_id)
}

pub fn compute_backoff(attempt: i64) -> std::time::Duration {
    let base_secs = RETRY_BASE_SECS;
    let multiplier = 2u64.pow(attempt as u32);
    let interval_secs = (base_secs * multiplier).min(RETRY_MAX_SECS);
    let jitter_range = (interval_secs as f64 * 0.25) as u64;
    let jitter = if jitter_range > 0 {
        let r: u64 = rand::random::<u64>() % (jitter_range * 2);
        r.saturating_sub(jitter_range)
    } else {
        0
    };
    let final_secs = if attempt >= MAX_RETRY_ATTEMPTS {
        RETRY_MAX_SECS
    } else {
        interval_secs.saturating_add(jitter).min(RETRY_MAX_SECS)
    };
    std::time::Duration::from_secs(final_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_alert_state_variants() {
        assert_eq!(parse_alert_state("normal"), AlertState::Normal);
        assert_eq!(parse_alert_state("pending_fire"), AlertState::PendingFire);
        assert_eq!(parse_alert_state("firing"), AlertState::Firing);
        assert_eq!(parse_alert_state("pending_clear"), AlertState::PendingClear);
    }

    #[test]
    fn evaluate_condition_operators() {
        assert!(evaluate_condition(0.5, 0.1, "gte"));
        assert!(!evaluate_condition(0.05, 0.1, "gte"));
        assert!(evaluate_condition(0.5, 0.1, "gt"));
        assert!(!evaluate_condition(0.1, 0.1, "gt"));
        assert!(evaluate_condition(0.05, 0.1, "lt"));
        assert!(evaluate_condition(0.1, 0.1, "lte"));
    }

    #[test]
    fn parse_threshold_percentage() {
        let val = parse_threshold("10%", "measurement_loss_ratio");
        assert!((val - 0.1).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_threshold_latency() {
        let val = parse_threshold("500ms", "p95_latency");
        assert!((val - 500.0).abs() < 1.0);
    }

    #[test]
    fn parse_threshold_raw() {
        let val = parse_threshold("3", "consecutive_total_loss_rounds");
        assert!((val - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn invert_threshold_gte() {
        let val = invert_threshold(0.1, "gte");
        assert!((val - 0.09).abs() < f64::EPSILON);
    }

    #[test]
    fn invert_operator_gte() {
        assert_eq!(invert_operator("gte"), "lt");
        assert_eq!(invert_operator("gt"), "lte");
        assert_eq!(invert_operator("lt"), "gte");
        assert_eq!(invert_operator("lte"), "gt");
    }

    #[test]
    fn compute_backoff_increases() {
        let d1 = compute_backoff(0);
        let d2 = compute_backoff(1);
        let d3 = compute_backoff(2);
        assert!(d1 <= d2);
        assert!(d2 <= d3);
    }

    #[test]
    fn compute_backoff_max() {
        let d = compute_backoff(15);
        assert!(d <= std::time::Duration::from_secs(RETRY_MAX_SECS + 360));
    }

    #[test]
    fn compute_metric_measurement_loss_ratio() {
        let rounds = vec![make_round(10, 8, 0, 2), make_round(10, 5, 0, 5)];
        let result = compute_metric("measurement_loss_ratio", &rounds);
        match result {
            MetricResult::Value(v) => assert!((v - 0.35).abs() < 0.01),
            _ => panic!("expected value"),
        }
    }

    #[test]
    fn compute_metric_health_failure_ratio() {
        let rounds = vec![make_round(10, 7, 2, 1), make_round(10, 6, 3, 1)];
        let result = compute_metric("health_failure_ratio", &rounds);
        match result {
            MetricResult::Value(v) => assert!((v - 0.25).abs() < 0.01),
            _ => panic!("expected value"),
        }
    }

    #[test]
    fn compute_metric_healthy_sample_ratio() {
        let rounds = vec![make_round(10, 8, 1, 1)];
        let result = compute_metric("healthy_sample_ratio", &rounds);
        match result {
            MetricResult::Value(v) => assert!((v - 0.8).abs() < 0.01),
            _ => panic!("expected value"),
        }
    }

    #[test]
    fn compute_metric_zero_attempted_is_insufficient() {
        let rounds = vec![make_round(0, 0, 0, 0)];
        let result = compute_metric("measurement_loss_ratio", &rounds);
        assert!(matches!(result, MetricResult::InsufficientData));
    }

    #[test]
    fn compute_metric_consecutive_total_loss() {
        let mut rounds = vec![];
        for _ in 0..3 {
            rounds.push(make_round(5, 0, 0, 5));
        }
        let result = compute_metric("consecutive_total_loss_rounds", &rounds);
        match result {
            MetricResult::Value(v) => assert_eq!(v, 3.0),
            _ => panic!("expected value"),
        }
    }

    #[test]
    fn compute_metric_consecutive_total_loss_broken_by_healthy() {
        let mut rounds = vec![];
        rounds.push(make_round(5, 5, 0, 0));
        for _ in 0..3 {
            rounds.push(make_round(5, 0, 0, 5));
        }
        let result = compute_metric("consecutive_total_loss_rounds", &rounds);
        match result {
            MetricResult::Value(v) => assert_eq!(v, 3.0),
            _ => panic!("expected value"),
        }
    }

    #[test]
    fn compute_metric_consecutive_unhealthy() {
        let rounds = vec![
            make_round(5, 3, 2, 0),
            make_round(5, 2, 3, 0),
            make_round(5, 4, 1, 0),
        ];
        let result = compute_metric("consecutive_unhealthy_rounds", &rounds);
        match result {
            MetricResult::Value(v) => assert_eq!(v, 3.0),
            _ => panic!("expected value"),
        }
    }

    #[test]
    fn should_transition_immediately_without_duration() {
        let rule = make_rule("r1", "10%", "5m", None);
        assert!(should_transition_to_firing(
            &rule,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:00:01Z"
        ));
    }

    #[test]
    fn should_not_transition_before_duration() {
        let rule = make_rule("r1", "10%", "5m", Some("2m"));
        assert!(!should_transition_to_firing(
            &rule,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:01:30Z"
        ));
    }

    #[test]
    fn should_transition_after_duration() {
        let rule = make_rule("r1", "10%", "5m", Some("2m"));
        assert!(should_transition_to_firing(
            &rule,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:02:01Z"
        ));
    }

    #[test]
    fn should_repeat_after_interval() {
        let rule = make_rule_with_repeat("r1", "10%", "5m", "15m");
        assert!(should_repeat_notification(
            &rule,
            Some("2024-01-01T00:00:00Z"),
            "2024-01-01T00:15:01Z"
        ));
    }

    #[test]
    fn should_not_repeat_before_interval() {
        let rule = make_rule_with_repeat("r1", "10%", "5m", "15m");
        assert!(!should_repeat_notification(
            &rule,
            Some("2024-01-01T00:00:00Z"),
            "2024-01-01T00:10:00Z"
        ));
    }

    #[test]
    fn hysteresis_clear_threshold() {
        let _rule = make_rule_with_clear("r1", "10%", "5m", "5%");
        let clear = parse_threshold("5%", "measurement_loss_ratio");
        assert!((clear - 0.05).abs() < f64::EPSILON);
    }

    fn make_round(
        attempted: i32,
        healthy: i32,
        unhealthy: i32,
        measurement_loss: i32,
    ) -> kemuri_storage::RoundRow {
        kemuri_storage::RoundRow {
            internal_id: 0,
            check_internal_id: 1,
            observer_internal_id: 1,
            scheduled_at: "2024-01-01T00:00:00Z".to_owned(),
            started_at: None,
            finished_at: None,
            execution_status: "complete".to_owned(),
            stop_reason: None,
            configured_samples: attempted,
            attempted_samples: attempted,
            latency_bearing_samples: healthy,
            healthy_samples: healthy,
            unhealthy_samples: unhealthy,
            measurement_loss_samples: measurement_loss,
            min_latency_ns: None,
            median_latency_ns: None,
            max_latency_ns: None,
            sample_blob: None,
            outcome_summary: None,
            config_generation: None,
            check_revision_id: None,
            created_at: "2024-01-01T00:00:00Z".to_owned(),
        }
    }

    fn make_rule(
        id: &str,
        threshold: &str,
        window: &str,
        duration: Option<&str>,
    ) -> AlertRuleConfig {
        AlertRuleConfig {
            id: kemuri_core::RuleId::new(id).unwrap(),
            profile: kemuri_core::ProfileId::new("p1").unwrap(),
            metric: "measurement_loss_ratio".to_owned(),
            operator: "gte".to_owned(),
            threshold: threshold.to_owned(),
            window: window.to_owned(),
            notifier: kemuri_core::NotifierId::new("n1").unwrap(),
            duration: duration.map(|s| s.to_owned()),
            clear_threshold: None,
            clear_operator: None,
            repeat_every: None,
            minimum_rounds: None,
            no_data_period: None,
        }
    }

    fn make_rule_with_repeat(
        id: &str,
        threshold: &str,
        window: &str,
        repeat: &str,
    ) -> AlertRuleConfig {
        AlertRuleConfig {
            id: kemuri_core::RuleId::new(id).unwrap(),
            profile: kemuri_core::ProfileId::new("p1").unwrap(),
            metric: "measurement_loss_ratio".to_owned(),
            operator: "gte".to_owned(),
            threshold: threshold.to_owned(),
            window: window.to_owned(),
            notifier: kemuri_core::NotifierId::new("n1").unwrap(),
            duration: None,
            clear_threshold: None,
            clear_operator: None,
            repeat_every: Some(repeat.to_owned()),
            minimum_rounds: None,
            no_data_period: None,
        }
    }

    fn make_rule_with_clear(
        id: &str,
        threshold: &str,
        window: &str,
        clear_threshold: &str,
    ) -> AlertRuleConfig {
        AlertRuleConfig {
            id: kemuri_core::RuleId::new(id).unwrap(),
            profile: kemuri_core::ProfileId::new("p1").unwrap(),
            metric: "measurement_loss_ratio".to_owned(),
            operator: "gte".to_owned(),
            threshold: threshold.to_owned(),
            window: window.to_owned(),
            notifier: kemuri_core::NotifierId::new("n1").unwrap(),
            duration: None,
            clear_threshold: Some(clear_threshold.to_owned()),
            clear_operator: None,
            repeat_every: None,
            minimum_rounds: None,
            no_data_period: None,
        }
    }
}
