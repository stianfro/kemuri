use sqlx::SqlitePool;

use super::RollupRow;

pub struct RollupRepo;

impl RollupRepo {
    pub async fn upsert(
        pool: &SqlitePool,
        rollup: &super::InsertRollup,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO rollups (check_internal_id, observer_internal_id, resolution_seconds, bucket_start, scheduled_rounds, completed_rounds, partial_rounds, configured_sample_slots, attempted_samples, latency_bearing_samples, healthy_samples, unhealthy_samples, measurement_loss_samples, outcome_counts, min_latency_ns, max_latency_ns, sum_latency_ns, histogram_version, histogram_blob, no_data_counts) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) ON CONFLICT (check_internal_id, observer_internal_id, resolution_seconds, bucket_start) DO UPDATE SET scheduled_rounds = excluded.scheduled_rounds, completed_rounds = excluded.completed_rounds, partial_rounds = excluded.partial_rounds, configured_sample_slots = excluded.configured_sample_slots, attempted_samples = excluded.attempted_samples, latency_bearing_samples = excluded.latency_bearing_samples, healthy_samples = excluded.healthy_samples, unhealthy_samples = excluded.unhealthy_samples, measurement_loss_samples = excluded.measurement_loss_samples, outcome_counts = excluded.outcome_counts, min_latency_ns = excluded.min_latency_ns, max_latency_ns = excluded.max_latency_ns, sum_latency_ns = excluded.sum_latency_ns, histogram_version = excluded.histogram_version, histogram_blob = excluded.histogram_blob, no_data_counts = excluded.no_data_counts",
        )
        .bind(rollup.check_internal_id)
        .bind(rollup.observer_internal_id)
        .bind(rollup.resolution_seconds)
        .bind(&rollup.bucket_start)
        .bind(rollup.scheduled_rounds)
        .bind(rollup.completed_rounds)
        .bind(rollup.partial_rounds)
        .bind(rollup.configured_sample_slots)
        .bind(rollup.attempted_samples)
        .bind(rollup.latency_bearing_samples)
        .bind(rollup.healthy_samples)
        .bind(rollup.unhealthy_samples)
        .bind(rollup.measurement_loss_samples)
        .bind(&rollup.outcome_counts)
        .bind(rollup.min_latency_ns)
        .bind(rollup.max_latency_ns)
        .bind(rollup.sum_latency_ns)
        .bind(rollup.histogram_version)
        .bind(&rollup.histogram_blob)
        .bind(&rollup.no_data_counts)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn query_by_check_and_range(
        pool: &SqlitePool,
        check_internal_id: i64,
        observer_internal_id: i64,
        resolution_seconds: i64,
        from: &str,
        to: &str,
    ) -> Result<Vec<RollupRow>, sqlx::Error> {
        sqlx::query_as::<_, RollupRow>(
            "SELECT check_internal_id, observer_internal_id, resolution_seconds, bucket_start,
                    scheduled_rounds, completed_rounds, partial_rounds,
                    configured_sample_slots, attempted_samples, latency_bearing_samples,
                    healthy_samples, unhealthy_samples, measurement_loss_samples,
                    outcome_counts, min_latency_ns, max_latency_ns, sum_latency_ns,
                    histogram_version, histogram_blob, no_data_counts
             FROM rollups
             WHERE check_internal_id = ? AND observer_internal_id = ?
               AND resolution_seconds = ?
               AND unixepoch(bucket_start) >= unixepoch(?)
               AND unixepoch(bucket_start) < unixepoch(?)
             ORDER BY unixepoch(bucket_start) ASC",
        )
        .bind(check_internal_id)
        .bind(observer_internal_id)
        .bind(resolution_seconds)
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await
    }

    pub async fn get_latest_bucket_start(
        pool: &SqlitePool,
        check_internal_id: i64,
        observer_internal_id: i64,
        resolution_seconds: i64,
    ) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(String,)> = sqlx::query_as(
            "SELECT bucket_start FROM rollups WHERE check_internal_id = ? AND observer_internal_id = ? AND resolution_seconds = ? ORDER BY bucket_start DESC LIMIT 1",
        )
        .bind(check_internal_id)
        .bind(observer_internal_id)
        .bind(resolution_seconds)
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|(s,)| s))
    }

    pub async fn get_earliest_bucket_start(
        pool: &SqlitePool,
        resolution_seconds: i64,
    ) -> Result<Option<String>, sqlx::Error> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT MIN(bucket_start) FROM rollups WHERE resolution_seconds = ?")
                .bind(resolution_seconds)
                .fetch_optional(pool)
                .await?;
        Ok(row.and_then(|(s,)| if s.is_empty() { None } else { Some(s) }))
    }

    pub async fn delete_before(
        pool: &SqlitePool,
        resolution_seconds: i64,
        before: &str,
        batch_size: i64,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM rollups
             WHERE rowid IN (
                 SELECT rowid FROM rollups
                 WHERE resolution_seconds = ? AND bucket_start < ?
                 ORDER BY bucket_start
                 LIMIT ?
             )",
        )
        .bind(resolution_seconds)
        .bind(before)
        .bind(batch_size)
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn delete_before_simple(
        pool: &SqlitePool,
        resolution_seconds: i64,
        before: &str,
    ) -> Result<u64, sqlx::Error> {
        let result =
            sqlx::query("DELETE FROM rollups WHERE resolution_seconds = ? AND bucket_start < ?")
                .bind(resolution_seconds)
                .bind(before)
                .execute(pool)
                .await?;
        Ok(result.rows_affected())
    }

    pub async fn bucket_exists(
        pool: &SqlitePool,
        check_internal_id: i64,
        observer_internal_id: i64,
        resolution_seconds: i64,
        bucket_start: &str,
    ) -> Result<bool, sqlx::Error> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM rollups WHERE check_internal_id = ? AND observer_internal_id = ? AND resolution_seconds = ? AND bucket_start = ?",
        )
        .bind(check_internal_id)
        .bind(observer_internal_id)
        .bind(resolution_seconds)
        .bind(bucket_start)
        .fetch_optional(pool)
        .await?;
        Ok(row.is_some())
    }
}
