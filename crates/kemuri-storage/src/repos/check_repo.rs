use sqlx::SqlitePool;

use super::CheckRow;

pub struct CheckRepo;

impl CheckRepo {
    pub async fn upsert(
        pool: &SqlitePool,
        target_internal_id: i64,
        check_id: &str,
        probe_type: &str,
        revision_id: Option<&str>,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query_as::<_, CheckRow>(
            "SELECT internal_id, target_internal_id, check_id, probe_type, active, current_revision_id, first_seen_at, last_seen_at FROM checks WHERE target_internal_id = ? AND check_id = ?",
        )
        .bind(target_internal_id)
        .bind(check_id)
        .fetch_optional(pool)
        .await?;

        if let Some(existing) = row {
            sqlx::query(
                "UPDATE checks SET probe_type = ?, current_revision_id = ?, active = 1, last_seen_at = datetime('now') WHERE internal_id = ?",
            )
            .bind(probe_type)
            .bind(revision_id)
            .bind(existing.internal_id)
            .execute(pool)
            .await?;
            Ok(existing.internal_id)
        } else {
            let result = sqlx::query(
                "INSERT INTO checks (target_internal_id, check_id, probe_type, current_revision_id) VALUES (?, ?, ?, ?)",
            )
            .bind(target_internal_id)
            .bind(check_id)
            .bind(probe_type)
            .bind(revision_id)
            .execute(pool)
            .await?;
            Ok(result.last_insert_rowid())
        }
    }

    pub async fn deactivate(
        pool: &SqlitePool,
        target_internal_id: i64,
        check_id: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE checks SET active = 0 WHERE target_internal_id = ? AND check_id = ?")
            .bind(target_internal_id)
            .bind(check_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn get(
        pool: &SqlitePool,
        target_internal_id: i64,
        check_id: &str,
    ) -> Result<Option<CheckRow>, sqlx::Error> {
        sqlx::query_as::<_, CheckRow>(
            "SELECT internal_id, target_internal_id, check_id, probe_type, active, current_revision_id, first_seen_at, last_seen_at FROM checks WHERE target_internal_id = ? AND check_id = ?",
        )
        .bind(target_internal_id)
        .bind(check_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn list_active_by_target(
        pool: &SqlitePool,
        target_internal_id: i64,
    ) -> Result<Vec<CheckRow>, sqlx::Error> {
        sqlx::query_as::<_, CheckRow>(
            "SELECT internal_id, target_internal_id, check_id, probe_type, active, current_revision_id, first_seen_at, last_seen_at FROM checks WHERE target_internal_id = ? AND active = 1",
        )
        .bind(target_internal_id)
        .fetch_all(pool)
        .await
    }

    pub async fn get_internal_id(
        pool: &SqlitePool,
        target_internal_id: i64,
        check_id: &str,
    ) -> Result<Option<i64>, sqlx::Error> {
        let row: Option<(i64,)> = sqlx::query_as(
            "SELECT internal_id FROM checks WHERE target_internal_id = ? AND check_id = ?",
        )
        .bind(target_internal_id)
        .bind(check_id)
        .fetch_optional(pool)
        .await?;
        Ok(row.map(|(id,)| id))
    }

    pub async fn get_by_internal_id(
        pool: &SqlitePool,
        internal_id: i64,
    ) -> Result<Option<CheckRow>, sqlx::Error> {
        sqlx::query_as::<_, CheckRow>(
            "SELECT internal_id, target_internal_id, check_id, probe_type, active, current_revision_id, first_seen_at, last_seen_at FROM checks WHERE internal_id = ?",
        )
        .bind(internal_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn list_with_state(
        pool: &SqlitePool,
        target_internal_id: i64,
        observer_internal_id: i64,
    ) -> Result<Vec<CheckWithState>, sqlx::Error> {
        sqlx::query_as::<_, CheckWithState>(
            "SELECT c.internal_id, c.target_internal_id, c.check_id, c.probe_type, c.active, c.current_revision_id, c.first_seen_at, c.last_seen_at, ccs.state, ccs.last_round_at, ccs.last_latency_ns, ccs.last_measurement_loss_ratio, ccs.last_health_failure_ratio FROM checks c LEFT JOIN check_current_state ccs ON ccs.check_internal_id = c.internal_id AND ccs.observer_internal_id = ? WHERE c.target_internal_id = ? AND c.active = 1 ORDER BY c.check_id",
        )
        .bind(observer_internal_id)
        .bind(target_internal_id)
        .fetch_all(pool)
        .await
    }

    pub async fn get_with_state(
        pool: &SqlitePool,
        target_internal_id: i64,
        check_id: &str,
        observer_internal_id: i64,
    ) -> Result<Option<CheckWithState>, sqlx::Error> {
        sqlx::query_as::<_, CheckWithState>(
            "SELECT c.internal_id, c.target_internal_id, c.check_id, c.probe_type, c.active, c.current_revision_id, c.first_seen_at, c.last_seen_at, ccs.state, ccs.last_round_at, ccs.last_latency_ns, ccs.last_measurement_loss_ratio, ccs.last_health_failure_ratio FROM checks c LEFT JOIN check_current_state ccs ON ccs.check_internal_id = c.internal_id AND ccs.observer_internal_id = ? WHERE c.target_internal_id = ? AND c.check_id = ? AND c.active = 1",
        )
        .bind(observer_internal_id)
        .bind(target_internal_id)
        .bind(check_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn list_active_with_target(
        pool: &SqlitePool,
    ) -> Result<Vec<(i64, String)>, sqlx::Error> {
        sqlx::query_as::<_, (i64, String)>(
            "SELECT c.internal_id, c.probe_type FROM checks c WHERE c.active = 1",
        )
        .fetch_all(pool)
        .await
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CheckWithState {
    pub internal_id: i64,
    pub target_internal_id: i64,
    pub check_id: String,
    pub probe_type: String,
    pub active: bool,
    pub current_revision_id: Option<String>,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub state: Option<String>,
    pub last_round_at: Option<String>,
    pub last_latency_ns: Option<i64>,
    pub last_measurement_loss_ratio: Option<f64>,
    pub last_health_failure_ratio: Option<f64>,
}
