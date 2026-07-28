use std::sync::Arc;

use chrono::{DateTime, Utc};
use kemuri_config::RetentionConfig;
use kemuri_storage::{RollupRepo, RoundRepo};
use sqlx::SqlitePool;
use tokio::sync::broadcast;

const DELETE_BATCH_SIZE: i64 = 1000;

pub struct RetentionWorker {
    pool: Arc<SqlitePool>,
    config: RetentionConfig,
}

impl RetentionWorker {
    pub fn new(pool: Arc<SqlitePool>, config: RetentionConfig) -> Self {
        Self { pool, config }
    }

    pub async fn run(self, mut shutdown_rx: broadcast::Receiver<()>) {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600));
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                _ = shutdown_rx.recv() => {
                    tracing::info!("retention worker shutting down");
                    return;
                }
            }

            if let Err(e) = self.run_cycle().await {
                tracing::error!(error = %e, "retention worker cycle failed");
                metrics::counter!("kemuri_retention_errors").increment(1);
            }
        }
    }

    async fn run_cycle(&self) -> Result<(), sqlx::Error> {
        self.enforce_raw_retention().await?;
        self.enforce_rollup_5m_retention().await?;
        self.enforce_rollup_1h_retention().await?;
        self.enforce_alert_events_retention().await?;
        Ok(())
    }

    async fn enforce_raw_retention(&self) -> Result<(), sqlx::Error> {
        let retention = match self.config.parse_raw_retention() {
            Some(d) => d,
            None => return Ok(()),
        };

        let cutoff = Utc::now() - chrono::Duration::from_std(retention).unwrap_or_default();
        let cutoff_str = cutoff.to_rfc3339();

        let five_min_watermark = RollupRepo::get_earliest_bucket_start(&self.pool, 300).await?;

        if let Some(ref wm) = five_min_watermark
            && let Ok(wm_time) = DateTime::parse_from_rfc3339(wm)
            && wm_time >= cutoff
        {
            return Ok(());
        }

        let mut total_deleted: u64 = 0;
        loop {
            let deleted = RoundRepo::delete_before_batch_with_rollup_check(
                &self.pool,
                &cutoff_str,
                DELETE_BATCH_SIZE,
            )
            .await?;
            total_deleted += deleted;
            if deleted == 0 {
                break;
            }
            tokio::task::yield_now().await;
        }

        if total_deleted > 0 {
            tracing::info!(deleted = total_deleted, "retention: deleted raw rounds");
            metrics::counter!("kemuri_retention_raw_deleted").increment(total_deleted);
        }

        Ok(())
    }

    async fn enforce_rollup_5m_retention(&self) -> Result<(), sqlx::Error> {
        let retention = match self.config.parse_rollup_5m_retention() {
            Some(d) => d,
            None => return Ok(()),
        };

        let cutoff = Utc::now() - chrono::Duration::from_std(retention).unwrap_or_default();
        let cutoff_str = cutoff.to_rfc3339();

        let one_hour_watermark = RollupRepo::get_earliest_bucket_start(&self.pool, 3600).await?;

        if let Some(ref wm) = one_hour_watermark
            && let Ok(wm_time) = DateTime::parse_from_rfc3339(wm)
            && wm_time >= cutoff
        {
            return Ok(());
        }

        let deleted = RollupRepo::delete_before_simple(&self.pool, 300, &cutoff_str).await?;
        if deleted > 0 {
            tracing::info!(deleted, "retention: deleted 5-minute rollups");
            metrics::counter!("kemuri_retention_rollup_5m_deleted").increment(deleted);
        }

        Ok(())
    }

    async fn enforce_rollup_1h_retention(&self) -> Result<(), sqlx::Error> {
        let retention = match self.config.parse_rollup_1h_retention() {
            Some(d) => d,
            None => return Ok(()),
        };

        let cutoff = Utc::now() - chrono::Duration::from_std(retention).unwrap_or_default();
        let cutoff_str = cutoff.to_rfc3339();

        let deleted = RollupRepo::delete_before_simple(&self.pool, 3600, &cutoff_str).await?;
        if deleted > 0 {
            tracing::info!(deleted, "retention: deleted 1-hour rollups");
            metrics::counter!("kemuri_retention_rollup_1h_deleted").increment(deleted);
        }

        Ok(())
    }

    async fn enforce_alert_events_retention(&self) -> Result<(), sqlx::Error> {
        let retention = match self.config.parse_alert_events_retention() {
            Some(d) => d,
            None => return Ok(()),
        };

        let cutoff = Utc::now() - chrono::Duration::from_std(retention).unwrap_or_default();
        let cutoff_str = cutoff.to_rfc3339();

        let result = sqlx::query("DELETE FROM alert_events WHERE occurred_at < ?")
            .bind(&cutoff_str)
            .execute(self.pool.as_ref())
            .await?;
        if result.rows_affected() > 0 {
            tracing::info!(
                deleted = result.rows_affected(),
                "retention: deleted alert events"
            );
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

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

    #[tokio::test]
    async fn forever_retention_deletes_nothing() {
        let pool = setup_pool().await;
        let forever_config = RetentionConfig {
            raw_rounds: "forever".to_owned(),
            rollup_5m: "forever".to_owned(),
            rollup_1h: "forever".to_owned(),
            alert_events: "forever".to_owned(),
            notification_records: "forever".to_owned(),
        };

        let worker = RetentionWorker::new(Arc::new(pool), forever_config);
        worker.run_cycle().await.unwrap();
    }

    #[tokio::test]
    async fn raw_retention_deletes_after_period() {
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
            scheduled_at: "2020-01-01T00:00:00Z".to_owned(),
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

        let rollup = kemuri_storage::InsertRollup {
            check_internal_id: check_id,
            observer_internal_id: observer_id,
            resolution_seconds: 300,
            bucket_start: "2020-01-01T00:00:00Z".to_owned(),
            scheduled_rounds: 1,
            completed_rounds: 1,
            partial_rounds: 0,
            configured_sample_slots: 1,
            attempted_samples: 1,
            latency_bearing_samples: 1,
            healthy_samples: 1,
            unhealthy_samples: 0,
            measurement_loss_samples: 0,
            outcome_counts: "{}".to_owned(),
            min_latency_ns: None,
            max_latency_ns: None,
            sum_latency_ns: 0,
            histogram_version: 1,
            histogram_blob: None,
            no_data_counts: "{}".to_owned(),
        };
        kemuri_storage::RollupRepo::upsert(&pool, &rollup)
            .await
            .unwrap();

        let count_before: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM rounds WHERE check_internal_id = ?")
                .bind(check_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count_before.0, 1);

        let config = RetentionConfig {
            raw_rounds: "1d".to_owned(),
            ..RetentionConfig::default()
        };
        let worker = RetentionWorker::new(Arc::new(pool.clone()), config);
        worker.run_cycle().await.unwrap();

        let count_after: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM rounds WHERE check_internal_id = ?")
                .bind(check_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count_after.0, 0);
    }

    #[tokio::test]
    async fn raw_not_deleted_without_rollup() {
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
            scheduled_at: "2020-01-01T00:00:00Z".to_owned(),
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

        let config = RetentionConfig {
            raw_rounds: "1d".to_owned(),
            ..RetentionConfig::default()
        };
        let worker = RetentionWorker::new(Arc::new(pool.clone()), config);
        worker.run_cycle().await.unwrap();

        let count_after: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM rounds WHERE check_internal_id = ?")
                .bind(check_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count_after.0, 1);
    }
}
