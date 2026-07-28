use sqlx::SqlitePool;

use super::{ConfigEventRow, InsertConfigEvent};

pub struct ConfigEventRepo;

impl ConfigEventRepo {
    pub async fn insert(pool: &SqlitePool, event: &InsertConfigEvent) -> Result<i64, sqlx::Error> {
        let result = sqlx::query(
            "INSERT INTO config_events (generation_hash, event_type, summary) VALUES (?, ?, ?)",
        )
        .bind(&event.generation_hash)
        .bind(&event.event_type)
        .bind(&event.summary)
        .execute(pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn list_latest(
        pool: &SqlitePool,
        limit: i64,
    ) -> Result<Vec<ConfigEventRow>, sqlx::Error> {
        sqlx::query_as::<_, ConfigEventRow>(
            "SELECT internal_id, generation_hash, event_type, summary, occurred_at FROM config_events ORDER BY occurred_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}
