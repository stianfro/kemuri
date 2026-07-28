use sqlx::SqlitePool;

use super::TargetRow;

pub struct TargetRepo;

impl TargetRepo {
    pub async fn upsert(
        pool: &SqlitePool,
        target_id: &str,
        name: &str,
        group_path: &str,
        labels: &str,
    ) -> Result<i64, sqlx::Error> {
        let row = sqlx::query_as::<_, TargetRow>(
            "SELECT internal_id, target_id, name, group_path, labels, active, first_seen_at, last_seen_at FROM targets WHERE target_id = ?",
        )
        .bind(target_id)
        .fetch_optional(pool)
        .await?;

        if let Some(existing) = row {
            sqlx::query(
                "UPDATE targets SET name = ?, group_path = ?, labels = ?, active = 1, last_seen_at = datetime('now') WHERE internal_id = ?",
            )
            .bind(name)
            .bind(group_path)
            .bind(labels)
            .bind(existing.internal_id)
            .execute(pool)
            .await?;
            Ok(existing.internal_id)
        } else {
            let result = sqlx::query(
                "INSERT INTO targets (target_id, name, group_path, labels) VALUES (?, ?, ?, ?)",
            )
            .bind(target_id)
            .bind(name)
            .bind(group_path)
            .bind(labels)
            .execute(pool)
            .await?;
            Ok(result.last_insert_rowid())
        }
    }

    pub async fn deactivate(pool: &SqlitePool, target_id: &str) -> Result<(), sqlx::Error> {
        sqlx::query("UPDATE targets SET active = 0 WHERE target_id = ?")
            .bind(target_id)
            .execute(pool)
            .await?;
        Ok(())
    }

    pub async fn list_active(pool: &SqlitePool) -> Result<Vec<TargetRow>, sqlx::Error> {
        sqlx::query_as::<_, TargetRow>(
            "SELECT internal_id, target_id, name, group_path, labels, active, first_seen_at, last_seen_at FROM targets WHERE active = 1 ORDER BY target_id",
        )
        .fetch_all(pool)
        .await
    }

    pub async fn get_by_target_id(
        pool: &SqlitePool,
        target_id: &str,
    ) -> Result<Option<TargetRow>, sqlx::Error> {
        sqlx::query_as::<_, TargetRow>(
            "SELECT internal_id, target_id, name, group_path, labels, active, first_seen_at, last_seen_at FROM targets WHERE target_id = ?",
        )
        .bind(target_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn get_by_internal_id(
        pool: &SqlitePool,
        internal_id: i64,
    ) -> Result<Option<TargetRow>, sqlx::Error> {
        sqlx::query_as::<_, TargetRow>(
            "SELECT internal_id, target_id, name, group_path, labels, active, first_seen_at, last_seen_at FROM targets WHERE internal_id = ?",
        )
        .bind(internal_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn list_with_state(
        pool: &SqlitePool,
        observer_internal_id: i64,
    ) -> Result<Vec<TargetWithState>, sqlx::Error> {
        sqlx::query_as::<_, TargetWithState>(
            "SELECT t.internal_id, t.target_id, t.name, t.group_path, t.labels, t.active, t.first_seen_at, t.last_seen_at, ccs.state, ccs.last_latency_ns, ccs.last_measurement_loss_ratio, ccs.last_health_failure_ratio FROM targets t LEFT JOIN checks c ON c.target_internal_id = t.internal_id AND c.active = 1 LEFT JOIN check_current_state ccs ON ccs.check_internal_id = c.internal_id AND ccs.observer_internal_id = ? WHERE t.active = 1 ORDER BY t.group_path, t.target_id",
        )
        .bind(observer_internal_id)
        .fetch_all(pool)
        .await
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TargetWithState {
    pub internal_id: i64,
    pub target_id: String,
    pub name: String,
    pub group_path: String,
    pub labels: String,
    pub active: bool,
    pub first_seen_at: String,
    pub last_seen_at: String,
    pub state: Option<String>,
    pub last_latency_ns: Option<i64>,
    pub last_measurement_loss_ratio: Option<f64>,
    pub last_health_failure_ratio: Option<f64>,
}
