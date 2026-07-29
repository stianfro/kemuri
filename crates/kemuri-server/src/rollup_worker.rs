use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use kemuri_core::{Histogram, decode_samples};
use kemuri_storage::{InsertRollup, RollupRepo, RoundRepo};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

const RESOLUTION_5M: i64 = 300;
const RESOLUTION_1H: i64 = 3600;
const BATCH_SIZE: usize = 100;
const LOOKBACK_BUCKETS: i64 = 2;

pub struct RollupWorker {
    pool: Arc<SqlitePool>,
    resolutions: Vec<i64>,
}

impl RollupWorker {
    pub fn new(pool: Arc<SqlitePool>) -> Self {
        Self {
            pool,
            resolutions: vec![RESOLUTION_5M, RESOLUTION_1H],
        }
    }

    pub async fn run(self, mut shutdown_rx: broadcast::Receiver<()>) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = shutdown_rx.recv() => {
                    tracing::info!("rollup worker shutting down");
                    return;
                }
            }

            match self.run_cycle().await {
                Ok(buckets_processed) => {
                    if let Some(suppressed) = crate::failure_log::recovery("rollup", "database") {
                        tracing::info!(
                            suppressed,
                            "rollup worker recovered after repeated failures"
                        );
                    }
                    metrics::counter!("kemuri_rollup_buckets_processed")
                        .increment(buckets_processed);
                    metrics::gauge!("kemuri_rollup_last_cycle_buckets")
                        .set(buckets_processed as f64);
                }
                Err(e) => {
                    if let Some(suppressed) = crate::failure_log::failure("rollup", "database") {
                        tracing::error!(error = %e, suppressed, "rollup worker cycle failed");
                    }
                    metrics::counter!("kemuri_rollup_errors").increment(1);
                }
            }
        }
    }

    async fn run_cycle(&self) -> Result<u64, sqlx::Error> {
        let pairs = RoundRepo::list_distinct_check_observer_pairs(&self.pool).await?;
        let mut total_processed: u64 = 0;

        for (check_id, observer_id) in &pairs {
            for &resolution in &self.resolutions {
                let processed = self
                    .process_pair_resolution(*check_id, *observer_id, resolution)
                    .await?;
                total_processed += processed;

                if total_processed >= BATCH_SIZE as u64 {
                    return Ok(total_processed);
                }
            }
        }

        Ok(total_processed)
    }

    async fn process_pair_resolution(
        &self,
        check_internal_id: i64,
        observer_internal_id: i64,
        resolution_seconds: i64,
    ) -> Result<u64, sqlx::Error> {
        let now = Utc::now();
        let latest_closed_ts = (now.timestamp() / resolution_seconds) * resolution_seconds;
        let latest_closed = format_utc_timestamp(latest_closed_ts);

        let watermark = RollupRepo::get_latest_bucket_start(
            &self.pool,
            check_internal_id,
            observer_internal_id,
            resolution_seconds,
        )
        .await?;

        let from = compute_from(&watermark, resolution_seconds);
        let rounds = RoundRepo::query_by_check_range_with_observer(
            &self.pool,
            check_internal_id,
            observer_internal_id,
            &from,
            &latest_closed,
        )
        .await?;

        if rounds.is_empty() {
            return Ok(0);
        }

        let mut buckets: BTreeMap<i64, Vec<&kemuri_storage::RoundRow>> = BTreeMap::new();
        for round in &rounds {
            if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&round.scheduled_at) {
                let bucket_ts = (t.timestamp() / resolution_seconds) * resolution_seconds;
                buckets.entry(bucket_ts).or_default().push(round);
            }
        }

        let mut processed: u64 = 0;
        for (bucket_ts, bucket_rounds) in &buckets {
            let bucket_start = format_utc_timestamp(*bucket_ts);

            let rollup = aggregate_bucket(
                check_internal_id,
                observer_internal_id,
                resolution_seconds,
                &bucket_start,
                bucket_rounds,
            );

            RollupRepo::upsert(&self.pool, &rollup).await?;
            processed += 1;

            if processed >= BATCH_SIZE as u64 {
                tokio::task::yield_now().await;
            }
        }

        Ok(processed)
    }
}

fn format_utc_timestamp(ts: i64) -> String {
    DateTime::from_timestamp(ts, 0)
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

fn compute_from(watermark: &Option<String>, resolution_seconds: i64) -> String {
    if let Some(wm) = watermark
        && let Ok(t) = chrono::DateTime::parse_from_rfc3339(wm)
    {
        let lookback_secs = resolution_seconds * LOOKBACK_BUCKETS;
        let from_ts = t.timestamp() - lookback_secs;
        return DateTime::from_timestamp(from_ts, 0)
            .unwrap_or_else(Utc::now)
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    }
    "1970-01-01T00:00:00Z".to_owned()
}

fn aggregate_bucket(
    check_internal_id: i64,
    observer_internal_id: i64,
    resolution_seconds: i64,
    bucket_start: &str,
    rounds: &[&kemuri_storage::RoundRow],
) -> InsertRollup {
    let mut scheduled_rounds: i64 = 0;
    let mut completed_rounds: i64 = 0;
    let mut partial_rounds: i64 = 0;
    let mut configured_sample_slots: i64 = 0;
    let mut attempted_samples: i64 = 0;
    let mut latency_bearing_samples: i64 = 0;
    let mut healthy_samples: i64 = 0;
    let mut unhealthy_samples: i64 = 0;
    let mut measurement_loss_samples: i64 = 0;
    let mut min_latency_ns: Option<i64> = None;
    let mut max_latency_ns: Option<i64> = None;
    let mut sum_latency_ns: i64 = 0;
    let mut histogram = Histogram::new();
    let mut outcome_counts: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();
    let mut no_data_counts: std::collections::HashMap<String, i64> =
        std::collections::HashMap::new();

    for round in rounds {
        scheduled_rounds += 1;
        match round.execution_status.as_str() {
            "complete" => completed_rounds += 1,
            "partial" => partial_rounds += 1,
            other => {
                no_data_counts
                    .entry(other.to_owned())
                    .and_modify(|c| *c += 1)
                    .or_insert(1);
            }
        }

        configured_sample_slots += round.configured_samples as i64;
        attempted_samples += round.attempted_samples as i64;
        latency_bearing_samples += round.latency_bearing_samples as i64;
        healthy_samples += round.healthy_samples as i64;
        unhealthy_samples += round.unhealthy_samples as i64;
        measurement_loss_samples += round.measurement_loss_samples as i64;

        if let Some(min) = round.min_latency_ns {
            min_latency_ns = Some(min_latency_ns.map_or(min, |m| m.min(min)));
        }
        if let Some(max) = round.max_latency_ns {
            max_latency_ns = Some(max_latency_ns.map_or(max, |m| m.max(max)));
        }

        if let Some(ref blob) = round.sample_blob
            && let Ok(records) = decode_samples(blob)
        {
            for record in &records {
                if let Some(lat_ns) = record.latency_ns {
                    histogram.record(lat_ns);
                    sum_latency_ns += lat_ns as i64;
                }
                let outcome_key = format!("{:?}", record.outcome);
                outcome_counts
                    .entry(outcome_key)
                    .and_modify(|c| *c += 1)
                    .or_insert(1);
            }
        }
    }

    let histogram_blob = if histogram.count() > 0 {
        Some(histogram.encode())
    } else {
        None
    };

    InsertRollup {
        check_internal_id,
        observer_internal_id,
        resolution_seconds,
        bucket_start: bucket_start.to_owned(),
        scheduled_rounds,
        completed_rounds,
        partial_rounds,
        configured_sample_slots,
        attempted_samples,
        latency_bearing_samples,
        healthy_samples,
        unhealthy_samples,
        measurement_loss_samples,
        outcome_counts: serde_json::to_string(&outcome_counts).unwrap_or_default(),
        min_latency_ns,
        max_latency_ns,
        sum_latency_ns,
        histogram_version: 1,
        histogram_blob,
        no_data_counts: serde_json::to_string(&no_data_counts).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[tokio::test]
    async fn rollup_aggregation_idempotent() {
        let pool = setup_pool().await;
        let target_id = kemuri_storage::TargetRepo::upsert(&pool, "t1", "t1", "", "{}")
            .await
            .unwrap();
        let check_id = kemuri_storage::CheckRepo::upsert(&pool, target_id, "c1", "icmp", None)
            .await
            .unwrap();
        let observer_id: i64 = sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let round1 = kemuri_storage::InsertRound {
            check_internal_id: check_id,
            observer_internal_id: observer_id,
            scheduled_at: "2024-01-01T00:00:00Z".to_owned(),
            started_at: Some("2024-01-01T00:00:00.001Z".to_owned()),
            finished_at: Some("2024-01-01T00:00:00.002Z".to_owned()),
            execution_status: "complete".to_owned(),
            stop_reason: None,
            configured_samples: 3,
            attempted_samples: 3,
            latency_bearing_samples: 3,
            healthy_samples: 3,
            unhealthy_samples: 0,
            measurement_loss_samples: 0,
            min_latency_ns: Some(1_000_000),
            median_latency_ns: Some(2_000_000),
            max_latency_ns: Some(3_000_000),
            sample_blob: None,
            outcome_summary: Some("3/3 healthy".to_owned()),
            config_generation: None,
            check_revision_id: None,
        };

        let round2 = kemuri_storage::InsertRound {
            check_internal_id: check_id,
            observer_internal_id: observer_id,
            scheduled_at: "2024-01-01T00:00:30Z".to_owned(),
            started_at: Some("2024-01-01T00:00:30.001Z".to_owned()),
            finished_at: Some("2024-01-01T00:00:30.002Z".to_owned()),
            execution_status: "complete".to_owned(),
            stop_reason: None,
            configured_samples: 3,
            attempted_samples: 3,
            latency_bearing_samples: 3,
            healthy_samples: 2,
            unhealthy_samples: 1,
            measurement_loss_samples: 0,
            min_latency_ns: Some(2_000_000),
            median_latency_ns: Some(3_000_000),
            max_latency_ns: Some(5_000_000),
            sample_blob: None,
            outcome_summary: Some("2/3 healthy".to_owned()),
            config_generation: None,
            check_revision_id: None,
        };

        kemuri_storage::RoundRepo::insert(&pool, &round1)
            .await
            .unwrap();
        kemuri_storage::RoundRepo::insert(&pool, &round2)
            .await
            .unwrap();

        let rounds = rounds_as_refs(&pool, check_id, observer_id).await;
        let rounds_refs: Vec<&kemuri_storage::RoundRow> = rounds.iter().collect();

        let rollup1 = aggregate_bucket(
            check_id,
            observer_id,
            300,
            "2024-01-01T00:00:00Z",
            &rounds_refs,
        );

        RollupRepo::upsert(&pool, &rollup1).await.unwrap();

        let rollup2 = aggregate_bucket(
            check_id,
            observer_id,
            300,
            "2024-01-01T00:00:00Z",
            &rounds_refs,
        );

        RollupRepo::upsert(&pool, &rollup2).await.unwrap();

        let rows = RollupRepo::query_by_check_and_range(
            &pool,
            check_id,
            observer_id,
            300,
            "2024-01-01T00:00:00Z",
            "2024-01-01T00:05:00Z",
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].scheduled_rounds, 2);
        assert_eq!(rows[0].completed_rounds, 2);
        assert_eq!(rows[0].healthy_samples, 5);
        assert_eq!(rows[0].unhealthy_samples, 1);
        assert_eq!(rows[0].min_latency_ns, Some(1_000_000));
        assert_eq!(rows[0].max_latency_ns, Some(5_000_000));
    }

    #[tokio::test]
    async fn rollup_counters_are_sums() {
        let pool = setup_pool().await;
        let target_id = kemuri_storage::TargetRepo::upsert(&pool, "t1", "t1", "", "{}")
            .await
            .unwrap();
        let check_id = kemuri_storage::CheckRepo::upsert(&pool, target_id, "c1", "icmp", None)
            .await
            .unwrap();
        let observer_id: i64 = sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let round = kemuri_storage::InsertRound {
            check_internal_id: check_id,
            observer_internal_id: observer_id,
            scheduled_at: "2024-01-01T00:00:00Z".to_owned(),
            started_at: None,
            finished_at: None,
            execution_status: "complete".to_owned(),
            stop_reason: None,
            configured_samples: 5,
            attempted_samples: 5,
            latency_bearing_samples: 4,
            healthy_samples: 3,
            unhealthy_samples: 1,
            measurement_loss_samples: 1,
            min_latency_ns: None,
            median_latency_ns: None,
            max_latency_ns: None,
            sample_blob: None,
            outcome_summary: None,
            config_generation: None,
            check_revision_id: None,
        };

        kemuri_storage::RoundRepo::insert(&pool, &round)
            .await
            .unwrap();

        let rounds = rounds_as_refs(&pool, check_id, observer_id).await;
        let rounds_refs: Vec<&kemuri_storage::RoundRow> = rounds.iter().collect();

        let rollup = aggregate_bucket(
            check_id,
            observer_id,
            300,
            "2024-01-01T00:00:00Z",
            &rounds_refs,
        );

        assert_eq!(rollup.configured_sample_slots, 5);
        assert_eq!(rollup.attempted_samples, 5);
        assert_eq!(rollup.latency_bearing_samples, 4);
        assert_eq!(rollup.healthy_samples, 3);
        assert_eq!(rollup.unhealthy_samples, 1);
        assert_eq!(rollup.measurement_loss_samples, 1);
    }

    #[tokio::test]
    async fn rollup_batch_processing() {
        let pool = setup_pool().await;
        let target_id = kemuri_storage::TargetRepo::upsert(&pool, "t1", "t1", "", "{}")
            .await
            .unwrap();
        let check_id = kemuri_storage::CheckRepo::upsert(&pool, target_id, "c1", "icmp", None)
            .await
            .unwrap();
        let observer_id: i64 = sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(&pool)
            .await
            .unwrap()
            .last_insert_rowid();

        for i in 0..5 {
            let scheduled = format!("2024-01-01T00:00:{:02}Z", i * 10);
            let round = kemuri_storage::InsertRound {
                check_internal_id: check_id,
                observer_internal_id: observer_id,
                scheduled_at: scheduled,
                started_at: None,
                finished_at: None,
                execution_status: "complete".to_owned(),
                stop_reason: None,
                configured_samples: 1,
                attempted_samples: 1,
                latency_bearing_samples: 1,
                healthy_samples: 1,
                unhealthy_samples: 0,
                measurement_loss_samples: 0,
                min_latency_ns: None,
                median_latency_ns: None,
                max_latency_ns: None,
                sample_blob: None,
                outcome_summary: None,
                config_generation: None,
                check_revision_id: None,
            };
            kemuri_storage::RoundRepo::insert(&pool, &round)
                .await
                .unwrap();
        }

        let rounds = rounds_as_refs(&pool, check_id, observer_id).await;
        let rounds_refs: Vec<&kemuri_storage::RoundRow> = rounds.iter().collect();

        let mut bucket_map: BTreeMap<i64, Vec<&kemuri_storage::RoundRow>> = BTreeMap::new();
        for round in &rounds_refs {
            if let Ok(t) = chrono::DateTime::parse_from_rfc3339(&round.scheduled_at) {
                let bucket_ts = (t.timestamp() / 300) * 300;
                bucket_map.entry(bucket_ts).or_default().push(*round);
            }
        }

        assert_eq!(bucket_map.len(), 1);

        for (bucket_ts, bucket_rounds) in &bucket_map {
            let bucket_start = format_utc_timestamp(*bucket_ts);
            let rollup = aggregate_bucket(check_id, observer_id, 300, &bucket_start, bucket_rounds);
            RollupRepo::upsert(&pool, &rollup).await.unwrap();
        }

        let rows = RollupRepo::query_by_check_and_range(
            &pool,
            check_id,
            observer_id,
            300,
            "2024-01-01T00:00:00Z",
            "2024-01-01T01:00:00Z",
        )
        .await
        .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].scheduled_rounds, 5);
    }

    async fn rounds_as_refs(
        pool: &SqlitePool,
        check_id: i64,
        observer_id: i64,
    ) -> Vec<kemuri_storage::RoundRow> {
        kemuri_storage::RoundRepo::query_by_check_range_with_observer(
            pool,
            check_id,
            observer_id,
            "1970-01-01T00:00:00Z",
            "2099-01-01T00:00:00Z",
        )
        .await
        .unwrap()
    }

    async fn setup_pool() -> SqlitePool {
        let options = sqlx::sqlite::SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("../kemuri-storage/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool
    }
}
