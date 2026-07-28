use sqlx::SqlitePool;

use super::{InsertRound, RoundRow};

pub struct RoundRepo;

impl RoundRepo {
    pub async fn insert(pool: &SqlitePool, round: &InsertRound) -> Result<i64, RoundInsertError> {
        let existing: Option<(i64,)> = sqlx::query_as(
            "SELECT internal_id FROM rounds WHERE check_internal_id = ? AND observer_internal_id = ? AND scheduled_at = ?",
        )
        .bind(round.check_internal_id)
        .bind(round.observer_internal_id)
        .bind(&round.scheduled_at)
        .fetch_optional(pool)
        .await
        .map_err(RoundInsertError::Db)?;

        if existing.is_some() {
            return Err(RoundInsertError::Duplicate);
        }

        let result = sqlx::query(
            "INSERT INTO rounds (check_internal_id, observer_internal_id, scheduled_at, started_at, finished_at, execution_status, stop_reason, configured_samples, attempted_samples, latency_bearing_samples, healthy_samples, unhealthy_samples, measurement_loss_samples, min_latency_ns, median_latency_ns, max_latency_ns, sample_blob, outcome_summary, config_generation, check_revision_id) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(round.check_internal_id)
        .bind(round.observer_internal_id)
        .bind(&round.scheduled_at)
        .bind(&round.started_at)
        .bind(&round.finished_at)
        .bind(&round.execution_status)
        .bind(&round.stop_reason)
        .bind(round.configured_samples)
        .bind(round.attempted_samples)
        .bind(round.latency_bearing_samples)
        .bind(round.healthy_samples)
        .bind(round.unhealthy_samples)
        .bind(round.measurement_loss_samples)
        .bind(round.min_latency_ns)
        .bind(round.median_latency_ns)
        .bind(round.max_latency_ns)
        .bind(&round.sample_blob)
        .bind(&round.outcome_summary)
        .bind(&round.config_generation)
        .bind(&round.check_revision_id)
        .execute(pool)
        .await
        .map_err(RoundInsertError::Db)?;

        Ok(result.last_insert_rowid())
    }

    pub async fn query_by_check_and_range(
        pool: &SqlitePool,
        check_internal_id: i64,
        from: &str,
        to: &str,
    ) -> Result<Vec<RoundRow>, sqlx::Error> {
        sqlx::query_as::<_, RoundRow>(
            "SELECT internal_id, check_internal_id, observer_internal_id, scheduled_at, started_at, finished_at, execution_status, stop_reason, configured_samples, attempted_samples, latency_bearing_samples, healthy_samples, unhealthy_samples, measurement_loss_samples, min_latency_ns, median_latency_ns, max_latency_ns, sample_blob, outcome_summary, config_generation, check_revision_id, created_at FROM rounds WHERE check_internal_id = ? AND scheduled_at >= ? AND scheduled_at < ? ORDER BY scheduled_at DESC",
        )
        .bind(check_internal_id)
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await
    }

    pub async fn get_latest(
        pool: &SqlitePool,
        check_internal_id: i64,
    ) -> Result<Option<RoundRow>, sqlx::Error> {
        sqlx::query_as::<_, RoundRow>(
            "SELECT internal_id, check_internal_id, observer_internal_id, scheduled_at, started_at, finished_at, execution_status, stop_reason, configured_samples, attempted_samples, latency_bearing_samples, healthy_samples, unhealthy_samples, measurement_loss_samples, min_latency_ns, median_latency_ns, max_latency_ns, sample_blob, outcome_summary, config_generation, check_revision_id, created_at FROM rounds WHERE check_internal_id = ? ORDER BY scheduled_at DESC LIMIT 1",
        )
        .bind(check_internal_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn query_by_check_range_with_observer(
        pool: &SqlitePool,
        check_internal_id: i64,
        observer_internal_id: i64,
        from: &str,
        to: &str,
    ) -> Result<Vec<RoundRow>, sqlx::Error> {
        sqlx::query_as::<_, RoundRow>(
            "SELECT internal_id, check_internal_id, observer_internal_id, scheduled_at, started_at, finished_at, execution_status, stop_reason, configured_samples, attempted_samples, latency_bearing_samples, healthy_samples, unhealthy_samples, measurement_loss_samples, min_latency_ns, median_latency_ns, max_latency_ns, sample_blob, outcome_summary, config_generation, check_revision_id, created_at FROM rounds WHERE check_internal_id = ? AND observer_internal_id = ? AND scheduled_at >= ? AND scheduled_at < ? ORDER BY scheduled_at ASC",
        )
        .bind(check_internal_id)
        .bind(observer_internal_id)
        .bind(from)
        .bind(to)
        .fetch_all(pool)
        .await
    }

    pub async fn query_recent_by_check(
        pool: &SqlitePool,
        check_internal_id: i64,
        observer_internal_id: i64,
        limit: i64,
        cursor: Option<&str>,
    ) -> Result<Vec<RoundRow>, sqlx::Error> {
        if let Some(cursor) = cursor {
            sqlx::query_as::<_, RoundRow>(
                "SELECT internal_id, check_internal_id, observer_internal_id, scheduled_at, started_at, finished_at, execution_status, stop_reason, configured_samples, attempted_samples, latency_bearing_samples, healthy_samples, unhealthy_samples, measurement_loss_samples, min_latency_ns, median_latency_ns, max_latency_ns, sample_blob, outcome_summary, config_generation, check_revision_id, created_at FROM rounds WHERE check_internal_id = ? AND observer_internal_id = ? AND scheduled_at < ? ORDER BY scheduled_at DESC LIMIT ?",
            )
            .bind(check_internal_id)
            .bind(observer_internal_id)
            .bind(cursor)
            .bind(limit)
            .fetch_all(pool)
            .await
        } else {
            sqlx::query_as::<_, RoundRow>(
                "SELECT internal_id, check_internal_id, observer_internal_id, scheduled_at, started_at, finished_at, execution_status, stop_reason, configured_samples, attempted_samples, latency_bearing_samples, healthy_samples, unhealthy_samples, measurement_loss_samples, min_latency_ns, median_latency_ns, max_latency_ns, sample_blob, outcome_summary, config_generation, check_revision_id, created_at FROM rounds WHERE check_internal_id = ? AND observer_internal_id = ? ORDER BY scheduled_at DESC LIMIT ?",
            )
            .bind(check_internal_id)
            .bind(observer_internal_id)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
    }

    pub async fn count_by_check_range(
        pool: &SqlitePool,
        check_internal_id: i64,
        observer_internal_id: i64,
        from: &str,
        to: &str,
    ) -> Result<i64, sqlx::Error> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM rounds WHERE check_internal_id = ? AND observer_internal_id = ? AND scheduled_at >= ? AND scheduled_at < ?",
        )
        .bind(check_internal_id)
        .bind(observer_internal_id)
        .bind(from)
        .bind(to)
        .fetch_one(pool)
        .await?;
        Ok(row.0)
    }

    pub async fn list_distinct_check_observer_pairs(
        pool: &SqlitePool,
    ) -> Result<Vec<(i64, i64)>, sqlx::Error> {
        sqlx::query_as::<_, (i64, i64)>(
            "SELECT DISTINCT check_internal_id, observer_internal_id FROM rounds",
        )
        .fetch_all(pool)
        .await
    }

    pub async fn has_rounds_since(
        pool: &SqlitePool,
        check_internal_id: i64,
        observer_internal_id: i64,
        since: &str,
    ) -> Result<bool, sqlx::Error> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT 1 FROM rounds WHERE check_internal_id = ? AND observer_internal_id = ? AND scheduled_at >= ? LIMIT 1",
        )
        .bind(check_internal_id)
        .bind(observer_internal_id)
        .bind(since)
        .fetch_optional(pool)
        .await?;
        Ok(row.is_some())
    }

    pub async fn delete_before_batch(
        pool: &SqlitePool,
        before: &str,
        batch_size: i64,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM rounds WHERE scheduled_at < ? AND internal_id IN (SELECT internal_id FROM rounds WHERE scheduled_at < ? LIMIT ?)",
        )
        .bind(before)
        .bind(before)
        .bind(batch_size)
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }

    pub async fn delete_before_batch_with_rollup_check(
        pool: &SqlitePool,
        before: &str,
        batch_size: i64,
    ) -> Result<u64, sqlx::Error> {
        let result = sqlx::query(
            "DELETE FROM rounds WHERE scheduled_at < ? AND internal_id IN (
                SELECT r.internal_id FROM rounds r
                WHERE r.scheduled_at < ? AND EXISTS (
                    SELECT 1 FROM rollups ru
                    WHERE ru.check_internal_id = r.check_internal_id
                      AND ru.observer_internal_id = r.observer_internal_id
                      AND ru.resolution_seconds = 300
                      AND unixepoch(r.scheduled_at) >= unixepoch(ru.bucket_start)
                      AND unixepoch(r.scheduled_at) < unixepoch(ru.bucket_start) + 300
                )
                LIMIT ?
            )",
        )
        .bind(before)
        .bind(before)
        .bind(batch_size)
        .execute(pool)
        .await?;
        Ok(result.rows_affected())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RoundInsertError {
    #[error("duplicate round")]
    Duplicate,
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
}
