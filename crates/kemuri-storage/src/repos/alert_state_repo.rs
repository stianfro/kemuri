use sqlx::SqlitePool;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AlertStateRow {
    pub internal_id: i64,
    pub rule_id: String,
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub state: String,
    pub state_entered_at: String,
    pub first_condition_true_at: Option<String>,
    pub last_evaluated_at: Option<String>,
    pub last_notification_at: Option<String>,
    pub fingerprint: Option<String>,
    pub last_metric_value: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct UpsertAlertState {
    pub rule_id: String,
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub state: String,
    pub state_entered_at: String,
    pub first_condition_true_at: Option<String>,
    pub last_evaluated_at: Option<String>,
    pub last_notification_at: Option<String>,
    pub fingerprint: Option<String>,
    pub last_metric_value: Option<f64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AlertEventRow {
    pub internal_id: i64,
    pub rule_id: String,
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub event_type: String,
    pub from_state: String,
    pub to_state: String,
    pub metric_value: Option<f64>,
    pub threshold_value: Option<f64>,
    pub occurred_at: String,
}

#[derive(Debug, Clone)]
pub struct InsertAlertEvent {
    pub rule_id: String,
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub event_type: String,
    pub from_state: String,
    pub to_state: String,
    pub metric_value: Option<f64>,
    pub threshold_value: Option<f64>,
    pub occurred_at: String,
}

pub struct AlertStateRepo;

impl AlertStateRepo {
    pub async fn get(
        pool: &SqlitePool,
        rule_id: &str,
        check_internal_id: i64,
        observer_internal_id: i64,
    ) -> Result<Option<AlertStateRow>, sqlx::Error> {
        sqlx::query_as::<_, AlertStateRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, state, state_entered_at, first_condition_true_at, last_evaluated_at, last_notification_at, fingerprint, last_metric_value FROM alert_states WHERE rule_id = ? AND check_internal_id = ? AND observer_internal_id = ?",
        )
        .bind(rule_id)
        .bind(check_internal_id)
        .bind(observer_internal_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn upsert(pool: &SqlitePool, state: &UpsertAlertState) -> Result<i64, sqlx::Error> {
        let existing = sqlx::query_as::<_, AlertStateRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, state, state_entered_at, first_condition_true_at, last_evaluated_at, last_notification_at, fingerprint, last_metric_value FROM alert_states WHERE rule_id = ? AND check_internal_id = ? AND observer_internal_id = ?",
        )
        .bind(&state.rule_id)
        .bind(state.check_internal_id)
        .bind(state.observer_internal_id)
        .fetch_optional(pool)
        .await?;

        if let Some(row) = existing {
            sqlx::query(
                "UPDATE alert_states SET state = ?, state_entered_at = ?, first_condition_true_at = ?, last_evaluated_at = ?, last_notification_at = ?, fingerprint = ?, last_metric_value = ? WHERE internal_id = ?",
            )
            .bind(&state.state)
            .bind(&state.state_entered_at)
            .bind(&state.first_condition_true_at)
            .bind(&state.last_evaluated_at)
            .bind(&state.last_notification_at)
            .bind(&state.fingerprint)
            .bind(state.last_metric_value)
            .bind(row.internal_id)
            .execute(pool)
            .await?;
            Ok(row.internal_id)
        } else {
            let result = sqlx::query(
                "INSERT INTO alert_states (rule_id, check_internal_id, observer_internal_id, state, state_entered_at, first_condition_true_at, last_evaluated_at, last_notification_at, fingerprint, last_metric_value) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&state.rule_id)
            .bind(state.check_internal_id)
            .bind(state.observer_internal_id)
            .bind(&state.state)
            .bind(&state.state_entered_at)
            .bind(&state.first_condition_true_at)
            .bind(&state.last_evaluated_at)
            .bind(&state.last_notification_at)
            .bind(&state.fingerprint)
            .bind(state.last_metric_value)
            .execute(pool)
            .await?;
            Ok(result.last_insert_rowid())
        }
    }

    pub async fn list_by_state(
        pool: &SqlitePool,
        states: &[&str],
    ) -> Result<Vec<AlertStateRow>, sqlx::Error> {
        if states.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders: Vec<String> = states
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 1))
            .collect();
        let sql = format!(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, state, state_entered_at, first_condition_true_at, last_evaluated_at, last_notification_at, fingerprint, last_metric_value FROM alert_states WHERE state IN ({}) ORDER BY state_entered_at DESC",
            placeholders.join(", ")
        );
        let mut query = sqlx::query_as::<_, AlertStateRow>(&sql);
        for state in states {
            query = query.bind(*state);
        }
        query.fetch_all(pool).await
    }

    pub async fn list_all(pool: &SqlitePool) -> Result<Vec<AlertStateRow>, sqlx::Error> {
        sqlx::query_as::<_, AlertStateRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, state, state_entered_at, first_condition_true_at, last_evaluated_at, last_notification_at, fingerprint, last_metric_value FROM alert_states ORDER BY state_entered_at DESC",
        )
        .fetch_all(pool)
        .await
    }

    pub async fn list_by_rule_id(
        pool: &SqlitePool,
        rule_id: &str,
    ) -> Result<Vec<AlertStateRow>, sqlx::Error> {
        sqlx::query_as::<_, AlertStateRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, state, state_entered_at, first_condition_true_at, last_evaluated_at, last_notification_at, fingerprint, last_metric_value FROM alert_states WHERE rule_id = ? ORDER BY state_entered_at DESC",
        )
        .bind(rule_id)
        .fetch_all(pool)
        .await
    }

    pub async fn get_by_internal_id(
        pool: &SqlitePool,
        internal_id: i64,
    ) -> Result<Option<AlertStateRow>, sqlx::Error> {
        sqlx::query_as::<_, AlertStateRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, state, state_entered_at, first_condition_true_at, last_evaluated_at, last_notification_at, fingerprint, last_metric_value FROM alert_states WHERE internal_id = ?",
        )
        .bind(internal_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn list_by_check(
        pool: &SqlitePool,
        check_internal_id: i64,
    ) -> Result<Vec<AlertStateRow>, sqlx::Error> {
        sqlx::query_as::<_, AlertStateRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, state, state_entered_at, first_condition_true_at, last_evaluated_at, last_notification_at, fingerprint, last_metric_value FROM alert_states WHERE check_internal_id = ? ORDER BY state_entered_at DESC",
        )
        .bind(check_internal_id)
        .fetch_all(pool)
        .await
    }
}

pub struct AlertEventRepo;

impl AlertEventRepo {
    pub async fn insert(pool: &SqlitePool, event: &InsertAlertEvent) -> Result<i64, sqlx::Error> {
        let result = sqlx::query(
            "INSERT INTO alert_events (rule_id, check_internal_id, observer_internal_id, event_type, from_state, to_state, metric_value, threshold_value, occurred_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&event.rule_id)
        .bind(event.check_internal_id)
        .bind(event.observer_internal_id)
        .bind(&event.event_type)
        .bind(&event.from_state)
        .bind(&event.to_state)
        .bind(event.metric_value)
        .bind(event.threshold_value)
        .bind(&event.occurred_at)
        .execute(pool)
        .await?;
        Ok(result.last_insert_rowid())
    }

    pub async fn list_by_check(
        pool: &SqlitePool,
        check_internal_id: i64,
        limit: i64,
    ) -> Result<Vec<AlertEventRow>, sqlx::Error> {
        sqlx::query_as::<_, AlertEventRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, event_type, from_state, to_state, metric_value, threshold_value, occurred_at FROM alert_events WHERE check_internal_id = ? ORDER BY occurred_at DESC LIMIT ?",
        )
        .bind(check_internal_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }

    pub async fn list_by_rule(
        pool: &SqlitePool,
        rule_id: &str,
        limit: i64,
    ) -> Result<Vec<AlertEventRow>, sqlx::Error> {
        sqlx::query_as::<_, AlertEventRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, event_type, from_state, to_state, metric_value, threshold_value, occurred_at FROM alert_events WHERE rule_id = ? ORDER BY occurred_at DESC LIMIT ?",
        )
        .bind(rule_id)
        .bind(limit)
        .fetch_all(pool)
        .await
    }

    pub async fn list_recent(
        pool: &SqlitePool,
        limit: i64,
    ) -> Result<Vec<AlertEventRow>, sqlx::Error> {
        sqlx::query_as::<_, AlertEventRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, event_type, from_state, to_state, metric_value, threshold_value, occurred_at FROM alert_events ORDER BY occurred_at DESC LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
    }

    pub async fn get_by_internal_id(
        pool: &SqlitePool,
        internal_id: i64,
    ) -> Result<Option<AlertEventRow>, sqlx::Error> {
        sqlx::query_as::<_, AlertEventRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, event_type, from_state, to_state, metric_value, threshold_value, occurred_at FROM alert_events WHERE internal_id = ?",
        )
        .bind(internal_id)
        .fetch_optional(pool)
        .await
    }

    pub async fn list_by_check_range(
        pool: &SqlitePool,
        check_internal_id: i64,
        from: &str,
        to: &str,
        limit: i64,
    ) -> Result<Vec<AlertEventRow>, sqlx::Error> {
        sqlx::query_as::<_, AlertEventRow>(
            "SELECT internal_id, rule_id, check_internal_id, observer_internal_id, event_type, from_state, to_state, metric_value, threshold_value, occurred_at FROM alert_events WHERE check_internal_id = ? AND occurred_at >= ? AND occurred_at < ? ORDER BY occurred_at DESC LIMIT ?",
        )
        .bind(check_internal_id)
        .bind(from)
        .bind(to)
        .bind(limit)
        .fetch_all(pool)
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CheckRepo, InsertNotificationOutbox, NotificationOutboxRepo, TargetRepo};
    use std::str::FromStr;

    async fn setup_pool() -> sqlx::SqlitePool {
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

    async fn setup_check(pool: &sqlx::SqlitePool, check_id: &str) -> i64 {
        let target_id = TargetRepo::upsert(pool, "t1", "t1", "", "{}")
            .await
            .unwrap();
        CheckRepo::upsert(pool, target_id, check_id, "icmp", None)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn alert_state_upsert_creates_and_updates() {
        let pool = setup_pool().await;
        let check_internal_id = setup_check(&pool, "c1").await;
        sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(&pool)
            .await
            .unwrap();

        let upsert = UpsertAlertState {
            rule_id: "r1".to_owned(),
            check_internal_id,
            observer_internal_id: 1,
            state: "normal".to_owned(),
            state_entered_at: "2024-01-01T00:00:00Z".to_owned(),
            first_condition_true_at: None,
            last_evaluated_at: Some("2024-01-01T00:01:00Z".to_owned()),
            last_notification_at: None,
            fingerprint: Some("r1:1".to_owned()),
            last_metric_value: Some(0.05),
        };
        let id = AlertStateRepo::upsert(&pool, &upsert).await.unwrap();
        assert!(id > 0);

        let row = AlertStateRepo::get(&pool, "r1", check_internal_id, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "normal");
        assert_eq!(row.rule_id, "r1");

        let updated = UpsertAlertState {
            state: "firing".to_owned(),
            last_metric_value: Some(0.5),
            ..upsert
        };
        AlertStateRepo::upsert(&pool, &updated).await.unwrap();

        let row = AlertStateRepo::get(&pool, "r1", check_internal_id, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "firing");
    }

    #[tokio::test]
    async fn alert_event_insert_and_list() {
        let pool = setup_pool().await;
        let check_internal_id = setup_check(&pool, "c1").await;
        sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(&pool)
            .await
            .unwrap();

        let event = InsertAlertEvent {
            rule_id: "r1".to_owned(),
            check_internal_id,
            observer_internal_id: 1,
            event_type: "firing".to_owned(),
            from_state: "normal".to_owned(),
            to_state: "firing".to_owned(),
            metric_value: Some(0.5),
            threshold_value: Some(0.1),
            occurred_at: "2024-01-01T00:05:00Z".to_owned(),
        };
        let id = AlertEventRepo::insert(&pool, &event).await.unwrap();
        assert!(id > 0);

        let events = AlertEventRepo::list_recent(&pool, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, "firing");
        assert_eq!(events[0].from_state, "normal");
        assert_eq!(events[0].to_state, "firing");
    }

    #[tokio::test]
    async fn notification_outbox_insert_and_mark() {
        let pool = setup_pool().await;
        let check_internal_id = setup_check(&pool, "c1").await;
        sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(&pool)
            .await
            .unwrap();

        let event = InsertAlertEvent {
            rule_id: "r1".to_owned(),
            check_internal_id,
            observer_internal_id: 1,
            event_type: "firing".to_owned(),
            from_state: "normal".to_owned(),
            to_state: "firing".to_owned(),
            metric_value: Some(0.5),
            threshold_value: Some(0.1),
            occurred_at: "2024-01-01T00:05:00Z".to_owned(),
        };
        let event_id = AlertEventRepo::insert(&pool, &event).await.unwrap();

        let entry = InsertNotificationOutbox {
            alert_event_internal_id: event_id,
            notifier_id: "slack".to_owned(),
            status: "pending".to_owned(),
            next_attempt_at: "2024-01-01T00:05:00Z".to_owned(),
        };
        let id = NotificationOutboxRepo::insert(&pool, &entry).await.unwrap();
        assert!(id > 0);

        let pending = NotificationOutboxRepo::list_pending(&pool, "2024-01-01T00:10:00Z", 10)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].notifier_id, "slack");

        NotificationOutboxRepo::mark_delivered(&pool, id)
            .await
            .unwrap();

        let pending_after = NotificationOutboxRepo::list_pending(&pool, "2024-01-01T00:10:00Z", 10)
            .await
            .unwrap();
        assert_eq!(pending_after.len(), 0);
    }

    #[tokio::test]
    async fn notification_outbox_retry_and_fail() {
        let pool = setup_pool().await;
        let check_internal_id = setup_check(&pool, "c1").await;
        sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(&pool)
            .await
            .unwrap();

        let event = InsertAlertEvent {
            rule_id: "r1".to_owned(),
            check_internal_id,
            observer_internal_id: 1,
            event_type: "firing".to_owned(),
            from_state: "normal".to_owned(),
            to_state: "firing".to_owned(),
            metric_value: Some(0.5),
            threshold_value: Some(0.1),
            occurred_at: "2024-01-01T00:05:00Z".to_owned(),
        };
        let event_id = AlertEventRepo::insert(&pool, &event).await.unwrap();

        let entry = InsertNotificationOutbox {
            alert_event_internal_id: event_id,
            notifier_id: "slack".to_owned(),
            status: "pending".to_owned(),
            next_attempt_at: "2024-01-01T00:05:00Z".to_owned(),
        };
        let id = NotificationOutboxRepo::insert(&pool, &entry).await.unwrap();

        NotificationOutboxRepo::mark_retry(&pool, id, "2024-01-01T00:06:00Z", "timeout")
            .await
            .unwrap();

        let pending = NotificationOutboxRepo::list_pending(&pool, "2024-01-01T00:07:00Z", 10)
            .await
            .unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].attempt_count, 1);

        NotificationOutboxRepo::mark_failed(&pool, id, "max retries exceeded")
            .await
            .unwrap();

        let pending_after = NotificationOutboxRepo::list_pending(&pool, "2099-01-01T00:00:00Z", 10)
            .await
            .unwrap();
        assert_eq!(pending_after.len(), 0);
    }

    #[tokio::test]
    async fn alert_state_survives_restart() {
        let pool = setup_pool().await;
        let check_internal_id = setup_check(&pool, "c1").await;
        sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(&pool)
            .await
            .unwrap();

        let upsert = UpsertAlertState {
            rule_id: "r1".to_owned(),
            check_internal_id,
            observer_internal_id: 1,
            state: "firing".to_owned(),
            state_entered_at: "2024-01-01T00:00:00Z".to_owned(),
            first_condition_true_at: Some("2024-01-01T00:00:00Z".to_owned()),
            last_evaluated_at: Some("2024-01-01T00:01:00Z".to_owned()),
            last_notification_at: Some("2024-01-01T00:01:00Z".to_owned()),
            fingerprint: Some("r1:1".to_owned()),
            last_metric_value: Some(0.5),
        };
        AlertStateRepo::upsert(&pool, &upsert).await.unwrap();

        let row = AlertStateRepo::get(&pool, "r1", check_internal_id, 1)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "firing");
        assert_eq!(
            row.last_notification_at,
            Some("2024-01-01T00:01:00Z".to_owned())
        );
    }

    #[tokio::test]
    async fn alert_state_list_by_state_filter() {
        let pool = setup_pool().await;
        let check1 = setup_check(&pool, "c1").await;
        let check2 = setup_check(&pool, "c2").await;
        sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(&pool)
            .await
            .unwrap();

        let firing = UpsertAlertState {
            rule_id: "r1".to_owned(),
            check_internal_id: check1,
            observer_internal_id: 1,
            state: "firing".to_owned(),
            state_entered_at: "2024-01-01T00:00:00Z".to_owned(),
            first_condition_true_at: None,
            last_evaluated_at: None,
            last_notification_at: None,
            fingerprint: None,
            last_metric_value: None,
        };
        AlertStateRepo::upsert(&pool, &firing).await.unwrap();

        let normal = UpsertAlertState {
            rule_id: "r2".to_owned(),
            check_internal_id: check2,
            observer_internal_id: 1,
            state: "normal".to_owned(),
            state_entered_at: "2024-01-01T00:00:00Z".to_owned(),
            first_condition_true_at: None,
            last_evaluated_at: None,
            last_notification_at: None,
            fingerprint: None,
            last_metric_value: None,
        };
        AlertStateRepo::upsert(&pool, &normal).await.unwrap();

        let firing_states = AlertStateRepo::list_by_state(&pool, &["firing"])
            .await
            .unwrap();
        assert_eq!(firing_states.len(), 1);
        assert_eq!(firing_states[0].rule_id, "r1");

        let active_states = AlertStateRepo::list_by_state(&pool, &["firing", "pending_fire"])
            .await
            .unwrap();
        assert_eq!(active_states.len(), 1);
    }
}
