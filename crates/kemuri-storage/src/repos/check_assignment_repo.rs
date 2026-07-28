use sqlx::SqlitePool;

use super::CheckAssignmentRow;

pub struct CheckAssignmentRepo;

impl CheckAssignmentRepo {
    pub async fn assign(
        pool: &SqlitePool,
        check_internal_id: i64,
        observer_internal_id: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO check_assignments (check_internal_id, observer_internal_id, active) VALUES (?, ?, 1) ON CONFLICT (check_internal_id, observer_internal_id) DO UPDATE SET active = 1, assigned_at = datetime('now')",
        )
        .bind(check_internal_id)
        .bind(observer_internal_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn unassign(
        pool: &SqlitePool,
        check_internal_id: i64,
        observer_internal_id: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE check_assignments SET active = 0 WHERE check_internal_id = ? AND observer_internal_id = ?",
        )
        .bind(check_internal_id)
        .bind(observer_internal_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn list_active_for_observer(
        pool: &SqlitePool,
        observer_internal_id: i64,
    ) -> Result<Vec<CheckAssignmentRow>, sqlx::Error> {
        sqlx::query_as::<_, CheckAssignmentRow>(
            "SELECT check_internal_id, observer_internal_id, active, assigned_at FROM check_assignments WHERE observer_internal_id = ? AND active = 1",
        )
        .bind(observer_internal_id)
        .fetch_all(pool)
        .await
    }
}
