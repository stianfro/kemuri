use sqlx::SqlitePool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct NotificationOutboxRow {
    pub internal_id: i64,
    pub alert_event_internal_id: i64,
    pub notifier_id: String,
    pub status: String,
    pub attempt_count: i64,
    pub next_attempt_at: String,
    pub last_attempt_at: Option<String>,
    pub last_error: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct InsertNotificationOutbox {
    pub alert_event_internal_id: i64,
    pub notifier_id: String,
    pub status: String,
    pub next_attempt_at: String,
}

pub struct NotificationOutboxRepo;

impl NotificationOutboxRepo {
    pub async fn insert(
        pool: &SqlitePool,
        entry: &InsertNotificationOutbox,
    ) -> Result<i64, sqlx::Error> {
        let result = sqlx::query(
            "INSERT INTO notification_outbox (alert_event_internal_id, notifier_id, status, next_attempt_at) VALUES (?, ?, ?, ?)",
        )
        .bind(entry.alert_event_internal_id)
        .bind(&entry.notifier_id)
        .bind(&entry.status)
        .bind(&entry.next_attempt_at)
        .execute(pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn list_pending(
        pool: &SqlitePool,
        now: &str,
        limit: i64,
    ) -> Result<Vec<NotificationOutboxRow>, sqlx::Error> {
        sqlx::query_as::<_, NotificationOutboxRow>(
            "SELECT internal_id, alert_event_internal_id, notifier_id, status, attempt_count, next_attempt_at, last_attempt_at, last_error, created_at FROM notification_outbox WHERE status = 'pending' AND next_attempt_at <= ? ORDER BY next_attempt_at ASC LIMIT ?",
        )
        .bind(now)
        .bind(limit)
        .fetch_all(pool)
        .await
    }

    pub async fn mark_delivered(pool: &SqlitePool, internal_id: i64) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE notification_outbox SET status = 'delivered', attempt_count = attempt_count + 1, last_attempt_at = datetime('now') WHERE internal_id = ?",
        )
        .bind(internal_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn mark_retry(
        pool: &SqlitePool,
        internal_id: i64,
        next_attempt_at: &str,
        error: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE notification_outbox SET attempt_count = attempt_count + 1, next_attempt_at = ?, last_attempt_at = datetime('now'), last_error = ? WHERE internal_id = ?",
        )
        .bind(next_attempt_at)
        .bind(error)
        .bind(internal_id)
        .execute(pool)
        .await?;
        Ok(())
    }

    pub async fn mark_failed(
        pool: &SqlitePool,
        internal_id: i64,
        error: &str,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE notification_outbox SET status = 'failed', attempt_count = attempt_count + 1, last_attempt_at = datetime('now'), last_error = ? WHERE internal_id = ?",
        )
        .bind(error)
        .bind(internal_id)
        .execute(pool)
        .await?;
        Ok(())
    }
}
