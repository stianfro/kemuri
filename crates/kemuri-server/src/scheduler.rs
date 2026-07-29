use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use kemuri_config::{ResolvedCheckDef, SchedulerConfig, StartupMode};
use kemuri_core::{CheckId, Clock, TargetId};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, mpsc};
use tokio::task::JoinHandle;

use crate::worker::RoundJob;
use crate::writer::RoundResult;

pub enum SchedulerCommand {
    Reconcile(Vec<ResolvedCheckDef>),
    Pause(bool),
}

pub struct Scheduler {
    config: SchedulerConfig,
    checks: Vec<ResolvedCheckDef>,
    queue: mpsc::Sender<RoundJob>,
    result_queue: mpsc::Sender<RoundResult>,
    running_rounds: Arc<Mutex<HashSet<(TargetId, CheckId)>>>,
    clock: Arc<dyn Clock>,
    shutdown_tx: Option<tokio::sync::watch::Sender<bool>>,
    cmd_tx: Option<mpsc::Sender<SchedulerCommand>>,
    handle: Option<JoinHandle<()>>,
    fatal_tx: Option<mpsc::Sender<&'static str>>,
}

impl Scheduler {
    pub fn new(
        config: SchedulerConfig,
        checks: Vec<ResolvedCheckDef>,
        queue: mpsc::Sender<RoundJob>,
        result_queue: mpsc::Sender<RoundResult>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            config,
            checks,
            queue,
            result_queue,
            running_rounds: Arc::new(Mutex::new(HashSet::new())),
            clock,
            shutdown_tx: None,
            cmd_tx: None,
            handle: None,
            fatal_tx: None,
        }
    }

    pub fn with_fatal_channel(mut self, sender: mpsc::Sender<&'static str>) -> Self {
        self.fatal_tx = Some(sender);
        self
    }

    pub fn start(&mut self) {
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);
        self.shutdown_tx = Some(shutdown_tx);

        let (cmd_tx, mut cmd_rx) = mpsc::channel::<SchedulerCommand>(16);
        self.cmd_tx = Some(cmd_tx);

        let checks = self.checks.clone();
        let queue = self.queue.clone();
        let result_queue = self.result_queue.clone();
        let running_rounds = self.running_rounds.clone();
        let clock = self.clock.clone();
        let max_concurrent = self.config.max_concurrent;
        let tick_interval_str = self.config.tick_interval.clone();
        let startup_mode = self.config.startup_mode;
        let jitter_ratio = self
            .config
            .default_jitter
            .trim_end_matches('%')
            .parse::<f64>()
            .unwrap_or(10.0)
            / 100.0;
        let probe_limits = self.config.max_concurrent_by_probe.clone();
        let fatal_tx = self.fatal_tx.clone();

        let handle = tokio::spawn(async move {
            let tick_interval =
                kemuri_core::parse_duration(&tick_interval_str).unwrap_or(Duration::from_secs(1));

            let mut next_due: HashMap<(TargetId, CheckId), DateTime<Utc>> = HashMap::new();
            let mut startup_rounds = HashSet::new();
            let mut active_checks = checks;
            let mut paused = false;

            let now: DateTime<Utc> = clock.system_time().into();
            for check in &active_checks {
                let key = (check.target_id.clone(), check.check_id.clone());
                let offset = deterministic_offset(
                    &check.target_id,
                    &check.check_id,
                    check.interval,
                    jitter_ratio,
                );
                let due = if startup_mode == StartupMode::ImmediateThenAligned {
                    startup_rounds.insert(key.clone());
                    now
                } else {
                    compute_next_due(now, check.interval, offset)
                };
                next_due.insert(key, due);
            }

            let mut tick = tokio::time::interval(tick_interval);
            'scheduler: loop {
                tokio::select! {
                    _ = tick.tick() => {}
                    _ = shutdown_rx.changed() => {
                        tracing::info!("scheduler shutting down");
                        break 'scheduler;
                    }
                    Some(cmd) = cmd_rx.recv() => {
                        match cmd {
                            SchedulerCommand::Reconcile(new_checks) => {
                                let old_keys: HashSet<(TargetId, CheckId)> = active_checks
                                    .iter()
                                    .map(|c| (c.target_id.clone(), c.check_id.clone()))
                                    .collect();
                                let new_keys: HashSet<(TargetId, CheckId)> = new_checks
                                    .iter()
                                    .map(|c| (c.target_id.clone(), c.check_id.clone()))
                                    .collect();

                                for key in old_keys.difference(&new_keys) {
                                    next_due.remove(key);
                                }

                                let now: DateTime<Utc> = clock.system_time().into();
                                for check in &new_checks {
                                    let key = (check.target_id.clone(), check.check_id.clone());
                                    if !old_keys.contains(&key) {
                                        let offset = deterministic_offset(
                                            &check.target_id,
                                            &check.check_id,
                                            check.interval,
                                            jitter_ratio,
                                        );
                                        let due = if startup_mode == StartupMode::ImmediateThenAligned {
                                            startup_rounds.insert(key.clone());
                                            now
                                        } else {
                                            compute_next_due(now, check.interval, offset)
                                        };
                                        next_due.insert(key, due);
                                    }
                                }

                                active_checks = new_checks;
                                metrics::gauge!("kemuri_scheduler_active_checks")
                                    .set(active_checks.len() as f64);
                            }
                            SchedulerCommand::Pause(value) => {
                                paused = value;
                                metrics::gauge!("kemuri_scheduler_paused")
                                    .set(if paused { 1.0 } else { 0.0 });
                            }
                        }
                        continue;
                    }
                }

                let now: DateTime<Utc> = clock.system_time().into();
                if paused {
                    continue;
                }

                for check in &active_checks {
                    let key = (check.target_id.clone(), check.check_id.clone());
                    let due = match next_due.get(&key) {
                        Some(d) => *d,
                        None => continue,
                    };

                    if due > now {
                        continue;
                    }

                    let is_running = running_rounds.lock().await.contains(&key);

                    if is_running {
                        tracing::debug!(
                            target_id = %check.target_id,
                            check_id = %check.check_id,
                            "skipping round due to overlap"
                        );
                        metrics::counter!("kemuri_scheduler_rounds_skipped_overlap",
                            "target_id" => check.target_id.to_string(),
                            "check_id" => check.check_id.to_string())
                        .increment(1);
                        record_skipped_round(
                            &result_queue,
                            check,
                            due,
                            kemuri_core::RoundExecutionStatus::SkippedOverlap,
                            "overlap",
                        );
                        let next = advance_due(due, check.interval);
                        next_due.insert(key, next);
                        continue;
                    }

                    let running_count = running_rounds.lock().await.len();

                    if running_count >= max_concurrent as usize {
                        tracing::warn!(
                            running_count,
                            max_concurrent,
                            "scheduler backpressure: max concurrent rounds reached"
                        );
                        metrics::counter!("kemuri_scheduler_backpressure").increment(1);
                        record_skipped_round(
                            &result_queue,
                            check,
                            due,
                            kemuri_core::RoundExecutionStatus::SkippedBackpressure,
                            "global_concurrency",
                        );
                        next_due.insert(key, advance_due(due, check.interval));
                        continue;
                    }
                    let probe_limit = match check.probe_kind {
                        kemuri_core::ProbeKind::Icmp => probe_limits.icmp,
                        kemuri_core::ProbeKind::Http => probe_limits.http,
                        kemuri_core::ProbeKind::Tcp => probe_limits.tcp,
                        kemuri_core::ProbeKind::Dns => probe_limits.dns,
                    };
                    if let Some(probe_limit) = probe_limit {
                        let running = running_rounds.lock().await;
                        let running_for_probe = running
                            .iter()
                            .filter(|key| {
                                active_checks.iter().any(|active| {
                                    active.target_id == key.0
                                        && active.check_id == key.1
                                        && active.probe_kind == check.probe_kind
                                })
                            })
                            .count();
                        if running_for_probe >= probe_limit as usize {
                            metrics::counter!(
                                "kemuri_scheduler_backpressure",
                                "probe_kind" => check.probe_kind.to_string()
                            )
                            .increment(1);
                            record_skipped_round(
                                &result_queue,
                                check,
                                due,
                                kemuri_core::RoundExecutionStatus::SkippedBackpressure,
                                "probe_concurrency",
                            );
                            next_due.insert(key, advance_due(due, check.interval));
                            continue;
                        }
                    }

                    running_rounds.lock().await.insert(key.clone());

                    let job = RoundJob {
                        target_id: check.target_id.clone(),
                        check_id: check.check_id.clone(),
                        check: check.clone(),
                        scheduled_at: due,
                    };

                    match queue.try_send(job) {
                        Ok(()) => {
                            metrics::counter!("kemuri_scheduler_rounds_total", "result" => "dispatched")
                                .increment(1);
                            let next = if startup_rounds.remove(&key) {
                                let offset = deterministic_offset(
                                    &check.target_id,
                                    &check.check_id,
                                    check.interval,
                                    jitter_ratio,
                                );
                                compute_next_due(now, check.interval, offset)
                            } else {
                                advance_due(due, check.interval)
                            };
                            next_due.insert(key, next);
                        }
                        Err(mpsc::error::TrySendError::Full(_)) => {
                            running_rounds.lock().await.remove(&key);
                            tracing::warn!(
                                target_id = %check.target_id,
                                check_id = %check.check_id,
                                "scheduler backpressure: queue full"
                            );
                            metrics::counter!("kemuri_scheduler_backpressure").increment(1);
                            metrics::counter!("kemuri_scheduler_rounds_total", "result" => "dropped")
                                .increment(1);
                            record_skipped_round(
                                &result_queue,
                                check,
                                due,
                                kemuri_core::RoundExecutionStatus::SkippedBackpressure,
                                "queue_full",
                            );
                            next_due.insert(key, advance_due(due, check.interval));
                        }
                        Err(mpsc::error::TrySendError::Closed(_)) => {
                            running_rounds.lock().await.remove(&key);
                            tracing::error!("scheduler queue closed, stopping");
                            break 'scheduler;
                        }
                    }
                }

                let queue_depth = queue.max_capacity() - queue.capacity();
                metrics::gauge!("kemuri_scheduler_queue_depth").set(queue_depth as f64);
            }
            if !*shutdown_rx.borrow()
                && let Some(sender) = fatal_tx
            {
                let _ = sender.try_send("scheduler");
            }
        });

        self.handle = Some(handle);
    }

    pub async fn stop(&mut self) {
        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(true);
        }
        if let Some(handle) = self.handle.take() {
            let _ = handle.await;
        }
        let (replacement, receiver) = mpsc::channel(1);
        self.queue = replacement;
        drop(receiver);
        let (replacement, receiver) = mpsc::channel(1);
        self.result_queue = replacement;
        drop(receiver);
    }

    pub async fn reconcile(&mut self, new_checks: Vec<ResolvedCheckDef>) {
        if let Some(ref cmd_tx) = self.cmd_tx {
            let _ = cmd_tx.send(SchedulerCommand::Reconcile(new_checks)).await;
        } else {
            self.checks = new_checks;
        }
    }

    pub fn running_rounds(&self) -> Arc<Mutex<HashSet<(TargetId, CheckId)>>> {
        self.running_rounds.clone()
    }

    pub fn queue_capacity(&self) -> usize {
        self.queue.capacity()
    }

    pub fn command_sender(&self) -> Option<mpsc::Sender<SchedulerCommand>> {
        self.cmd_tx.clone()
    }
}

fn record_skipped_round(
    result_queue: &mpsc::Sender<RoundResult>,
    check: &ResolvedCheckDef,
    scheduled_at: DateTime<Utc>,
    execution_status: kemuri_core::RoundExecutionStatus,
    reason: &str,
) {
    let now = Utc::now();
    let _ = result_queue.try_send(RoundResult {
        target_id: check.target_id.clone(),
        check_id: check.check_id.clone(),
        scheduled_at,
        started_at: now,
        finished_at: now,
        execution_status,
        stop_reason: Some(reason.to_owned()),
        sample_results: Vec::new(),
        configured_samples: match &check.probe_params {
            kemuri_config::ResolvedProbeParams::Icmp(params) => params.count,
            _ => 1,
        },
    });
}

fn deterministic_offset(
    target_id: &TargetId,
    check_id: &CheckId,
    interval: Duration,
    jitter_ratio: f64,
) -> Duration {
    let mut hasher = Sha256::new();
    hasher.update(target_id.as_str().as_bytes());
    hasher.update(check_id.as_str().as_bytes());
    let hash = hasher.finalize();
    let hash_val = u64::from_le_bytes(hash[..8].try_into().unwrap_or([0; 8]));
    let jitter_window = (interval.as_millis() as f64 * jitter_ratio) as u64;
    let offset_ms = hash_val % jitter_window.max(1);
    Duration::from_millis(offset_ms)
}

fn advance_due(due: DateTime<Utc>, interval: Duration) -> DateTime<Utc> {
    due + chrono::Duration::from_std(interval).unwrap_or_default()
}

fn compute_next_due(after: DateTime<Utc>, interval: Duration, offset: Duration) -> DateTime<Utc> {
    let interval_secs = interval.as_secs();
    if interval_secs == 0 {
        return after;
    }
    let after_ts = after.timestamp();
    let slot = after_ts / interval_secs as i64;
    let aligned = DateTime::from_timestamp(slot * interval_secs as i64, 0).unwrap_or(after);
    let candidate = aligned + chrono::Duration::from_std(offset).unwrap_or_default();
    if candidate > after {
        candidate
    } else {
        let next_aligned =
            DateTime::from_timestamp((slot + 1) * interval_secs as i64, 0).unwrap_or(after);
        next_aligned + chrono::Duration::from_std(offset).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_offset_is_stable() {
        let target = TargetId::new("t1").unwrap();
        let check = CheckId::new("c1").unwrap();
        let interval = Duration::from_secs(30);
        let o1 = deterministic_offset(&target, &check, interval, 0.1);
        let o2 = deterministic_offset(&target, &check, interval, 0.1);
        assert_eq!(o1, o2);
    }

    #[test]
    fn deterministic_offset_different_checks() {
        let target = TargetId::new("t1").unwrap();
        let c1 = CheckId::new("c1").unwrap();
        let c2 = CheckId::new("c2").unwrap();
        let interval = Duration::from_secs(30);
        let o1 = deterministic_offset(&target, &c1, interval, 0.1);
        let o2 = deterministic_offset(&target, &c2, interval, 0.1);
        assert_ne!(o1, o2);
    }

    #[test]
    fn compute_next_due_after_now() {
        let now = DateTime::parse_from_rfc3339("2024-01-01T00:00:10Z")
            .unwrap()
            .with_timezone(&Utc);
        let interval = Duration::from_secs(30);
        let offset = Duration::from_millis(100);
        let next = compute_next_due(now, interval, offset);
        assert!(next > now);
    }

    #[test]
    fn offset_within_jitter_window() {
        let target = TargetId::new("t1").unwrap();
        let check = CheckId::new("c1").unwrap();
        let interval = Duration::from_secs(30);
        let offset = deterministic_offset(&target, &check, interval, 0.1);
        let jitter_window = interval / 10;
        assert!(offset <= jitter_window);
    }
}
