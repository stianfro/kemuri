use sqlx::SqlitePool;

use super::{CheckCurrentStateRow, UpsertCheckCurrentState};

pub struct CheckCurrentStateRepo;

impl CheckCurrentStateRepo {
    pub async fn upsert(
        pool: &SqlitePool,
        state: &UpsertCheckCurrentState,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO check_current_state (check_internal_id, observer_internal_id, state, last_round_at, last_latency_ns, last_measurement_loss_ratio, last_health_failure_ratio) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT (check_internal_id, observer_internal_id) DO UPDATE SET state = excluded.state, last_round_at = excluded.last_round_at, last_latency_ns = excluded.last_latency_ns, last_measurement_loss_ratio = excluded.last_measurement_loss_ratio, last_health_failure_ratio = excluded.last_health_failure_ratio, updated_at = datetime('now')",
        )
        .bind(state.check_internal_id)
        .bind(state.observer_internal_id)
        .bind(&state.state)
        .bind(&state.last_round_at)
        .bind(state.last_latency_ns)
        .bind(state.last_measurement_loss_ratio)
        .bind(state.last_health_failure_ratio)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn get(
        pool: &SqlitePool,
        check_internal_id: i64,
        observer_internal_id: i64,
    ) -> Result<Option<CheckCurrentStateRow>, sqlx::Error> {
        sqlx::query_as::<_, CheckCurrentStateRow>(
            "SELECT check_internal_id, observer_internal_id, state, last_round_at, last_latency_ns, last_measurement_loss_ratio, last_health_failure_ratio, updated_at FROM check_current_state WHERE check_internal_id = ? AND observer_internal_id = ?",
        )
        .bind(check_internal_id)
        .bind(observer_internal_id)
        .fetch_optional(pool)
        .await
    }
}
