use std::collections::HashSet;

use sqlx::SqlitePool;

use kemuri_config::KemuriConfig;

#[derive(Debug, thiserror::Error)]
pub enum ReconciliationError {
    #[error("probe type mismatch for check {check_id}: existing={existing}, new={new}")]
    ProbeTypeMismatch {
        check_id: String,
        existing: String,
        new: String,
    },
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("configuration error: {0}")]
    Config(String),
}

pub async fn reconcile(
    pool: &SqlitePool,
    config: &KemuriConfig,
) -> Result<(), ReconciliationError> {
    reconcile_with_event(pool, config, "reconcile").await
}

pub async fn reconcile_with_event(
    pool: &SqlitePool,
    config: &KemuriConfig,
    event_type: &str,
) -> Result<(), ReconciliationError> {
    let resolved = config
        .resolve()
        .map_err(|error| ReconciliationError::Config(error.to_string()))?;
    let generation = resolved.generation.to_string();
    let mut transaction = pool.begin().await?;

    sqlx::query("INSERT OR IGNORE INTO observers (observer_id) VALUES ('local')")
        .execute(&mut *transaction)
        .await?;
    let observer_id: i64 =
        sqlx::query_scalar("SELECT internal_id FROM observers WHERE observer_id = 'local'")
            .fetch_one(&mut *transaction)
            .await?;

    let configured_targets: HashSet<String> = config
        .targets
        .iter()
        .filter(|target| target.enabled)
        .map(|target| target.id.to_string())
        .collect();
    let active_target_ids: Vec<String> =
        sqlx::query_scalar("SELECT target_id FROM targets WHERE active = 1")
            .fetch_all(&mut *transaction)
            .await?;
    for target_id in active_target_ids {
        if !configured_targets.contains(&target_id) {
            sqlx::query("UPDATE targets SET active = 0 WHERE target_id = ?")
                .bind(target_id)
                .execute(&mut *transaction)
                .await?;
        }
    }

    for target in &config.targets {
        let target_id = target.id.to_string();
        if !target.enabled {
            sqlx::query("UPDATE targets SET active = 0 WHERE target_id = ?")
                .bind(&target_id)
                .execute(&mut *transaction)
                .await?;
            continue;
        }
        let name = target.name.as_deref().unwrap_or(&target_id);
        let group_path = target.group_path.as_deref().unwrap_or("");
        let labels = serde_json::to_string(&target.labels.clone().unwrap_or_default())
            .unwrap_or_else(|_| "{}".to_owned());
        let target_internal_id: i64 = sqlx::query_scalar(
            "INSERT INTO targets (target_id, name, group_path, labels) VALUES (?, ?, ?, ?) \
             ON CONFLICT(target_id) DO UPDATE SET name = excluded.name, \
             group_path = excluded.group_path, labels = excluded.labels, active = 1, \
             last_seen_at = datetime('now') RETURNING internal_id",
        )
        .bind(&target_id)
        .bind(name)
        .bind(group_path)
        .bind(labels)
        .fetch_one(&mut *transaction)
        .await?;

        let active_checks: Vec<(i64, String, String)> = sqlx::query_as(
            "SELECT internal_id, check_id, probe_type FROM checks \
             WHERE target_internal_id = ? AND active = 1",
        )
        .bind(target_internal_id)
        .fetch_all(&mut *transaction)
        .await?;
        let configured_checks: HashSet<String> = target
            .checks
            .iter()
            .filter(|check| check.enabled)
            .map(|check| check.id.to_string())
            .collect();

        for (_, check_id, probe_type) in &active_checks {
            if let Some(check) = target
                .checks
                .iter()
                .find(|check| check.id.as_str() == check_id)
                && let Some(profile) = config
                    .profiles
                    .iter()
                    .find(|profile| profile.id() == &check.profile)
                && profile.kind().to_string() != *probe_type
            {
                return Err(ReconciliationError::ProbeTypeMismatch {
                    check_id: check_id.clone(),
                    existing: probe_type.clone(),
                    new: profile.kind().to_string(),
                });
            }
            if !configured_checks.contains(check_id) {
                sqlx::query(
                    "UPDATE checks SET active = 0 WHERE target_internal_id = ? AND check_id = ?",
                )
                .bind(target_internal_id)
                .bind(check_id)
                .execute(&mut *transaction)
                .await?;
            }
        }

        for check in &target.checks {
            let check_id = check.id.to_string();
            if !check.enabled {
                sqlx::query(
                    "UPDATE checks SET active = 0 WHERE target_internal_id = ? AND check_id = ?",
                )
                .bind(target_internal_id)
                .bind(&check_id)
                .execute(&mut *transaction)
                .await?;
                continue;
            }
            let resolved_check = resolved
                .checks
                .iter()
                .find(|item| item.target_id == target.id && item.check_id == check.id)
                .ok_or_else(|| {
                    ReconciliationError::Config(format!(
                        "missing resolved check {target_id}/{check_id}"
                    ))
                })?;
            let redacted = serde_json::json!({
                "probe_kind": resolved_check.probe_kind.to_string(),
                "interval_ms": resolved_check.interval.as_millis(),
                "timeout_ms": resolved_check.timeout.as_millis()
            })
            .to_string();
            let check_internal_id: i64 = sqlx::query_scalar(
                "INSERT INTO checks (target_internal_id, check_id, probe_type, current_revision_id, \
                 profile_id, config_generation, redacted_resolved_config, observer_assignment) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, 'local') \
                 ON CONFLICT(target_internal_id, check_id) DO UPDATE SET \
                 probe_type = excluded.probe_type, current_revision_id = excluded.current_revision_id, \
                 profile_id = excluded.profile_id, config_generation = excluded.config_generation, \
                 redacted_resolved_config = excluded.redacted_resolved_config, \
                 observer_assignment = 'local', active = 1, last_seen_at = datetime('now') \
                 RETURNING internal_id",
            )
            .bind(target_internal_id)
            .bind(&check_id)
            .bind(resolved_check.probe_kind.to_string())
            .bind(resolved_check.revision_id.as_str())
            .bind(check.profile.as_str())
            .bind(&generation)
            .bind(&redacted)
            .fetch_one(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT OR IGNORE INTO check_revisions \
                 (check_internal_id, revision_id, redacted_config) VALUES (?, ?, ?)",
            )
            .bind(check_internal_id)
            .bind(resolved_check.revision_id.as_str())
            .bind(&redacted)
            .execute(&mut *transaction)
            .await?;
            sqlx::query(
                "INSERT INTO check_assignments (check_internal_id, observer_internal_id, active) \
                 VALUES (?, ?, 1) ON CONFLICT(check_internal_id, observer_internal_id) \
                 DO UPDATE SET active = 1",
            )
            .bind(check_internal_id)
            .bind(observer_id)
            .execute(&mut *transaction)
            .await?;
        }
    }

    sqlx::query(
        "UPDATE check_assignments SET active = CASE WHEN EXISTS (
             SELECT 1 FROM checks c
             JOIN targets t ON t.internal_id = c.target_internal_id
             WHERE c.internal_id = check_assignments.check_internal_id
               AND c.active = 1 AND t.active = 1
         ) THEN 1 ELSE 0 END
         WHERE observer_internal_id = ?",
    )
    .bind(observer_id)
    .execute(&mut *transaction)
    .await?;

    let configured_rules: HashSet<&str> =
        config.rules.iter().map(|rule| rule.id.as_str()).collect();
    let alert_states: Vec<(i64, String, i64, i64, String, Option<bool>)> = sqlx::query_as(
        "SELECT a.internal_id, a.rule_id, a.check_internal_id,
                a.observer_internal_id, a.state, c.active
         FROM alert_states a
         LEFT JOIN checks c ON c.internal_id = a.check_internal_id
         WHERE a.state IN ('firing', 'pending_fire', 'pending_clear')",
    )
    .fetch_all(&mut *transaction)
    .await?;
    let now: String = sqlx::query_scalar("SELECT strftime('%Y-%m-%dT%H:%M:%fZ', 'now')")
        .fetch_one(&mut *transaction)
        .await?;
    for (internal_id, rule_id, check_id, alert_observer_id, state, check_active) in alert_states {
        if check_active == Some(true) && configured_rules.contains(rule_id.as_str()) {
            continue;
        }
        sqlx::query(
            "INSERT INTO alert_events
             (rule_id, check_internal_id, observer_internal_id, event_type,
              from_state, to_state, occurred_at, reason)
             VALUES (?, ?, ?, 'resolved', ?, 'normal', ?, 'config_removed')",
        )
        .bind(&rule_id)
        .bind(check_id)
        .bind(alert_observer_id)
        .bind(&state)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "UPDATE alert_states SET state = 'normal', state_entered_at = ?,
             first_condition_true_at = NULL, last_evaluated_at = ?
             WHERE internal_id = ?",
        )
        .bind(&now)
        .bind(&now)
        .bind(internal_id)
        .execute(&mut *transaction)
        .await?;
    }

    sqlx::query(
        "INSERT INTO config_events (generation_hash, event_type, summary) VALUES (?, ?, ?)",
    )
    .bind(&generation)
    .bind(event_type)
    .bind(format!("configuration {event_type}"))
    .execute(&mut *transaction)
    .await?;

    transaction.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::repos::TargetRepo;
    use sqlx::sqlite::SqliteConnectOptions;
    use sqlx::sqlite::SqlitePoolOptions;
    use std::str::FromStr;

    use super::*;

    async fn setup_pool() -> SqlitePool {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")
            .unwrap()
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn reconcile_inserts_new_target() {
        let pool = setup_pool().await;
        let config: KemuriConfig = serde_yaml::from_str(
            r#"
version: 1
profiles:
  - kind: icmp
    id: p1
targets:
  - id: t1
    address: 1.1.1.1
    checks:
      - id: c1
        profile: p1
"#,
        )
        .unwrap();
        reconcile(&pool, &config).await.unwrap();
        let target = TargetRepo::get_by_target_id(&pool, "t1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(target.target_id, "t1");
        assert!(target.active);
    }

    #[tokio::test]
    async fn reconcile_deactivates_removed_target() {
        let pool = setup_pool().await;

        let config1: KemuriConfig = serde_yaml::from_str(
            r#"
version: 1
targets:
  - id: t1
    address: 1.1.1.1
  - id: t2
    address: 2.2.2.2
"#,
        )
        .unwrap();
        reconcile(&pool, &config1).await.unwrap();

        let config2: KemuriConfig = serde_yaml::from_str(
            r#"
version: 1
targets:
  - id: t1
    address: 1.1.1.1
"#,
        )
        .unwrap();
        reconcile(&pool, &config2).await.unwrap();

        let t1 = TargetRepo::get_by_target_id(&pool, "t1")
            .await
            .unwrap()
            .unwrap();
        assert!(t1.active);
        let t2 = TargetRepo::get_by_target_id(&pool, "t2").await.unwrap();
        assert!(t2.is_none() || !t2.unwrap().active);
    }

    #[tokio::test]
    async fn reconcile_updates_target_metadata() {
        let pool = setup_pool().await;

        let config1: KemuriConfig = serde_yaml::from_str(
            r#"
version: 1
targets:
  - id: t1
    address: 1.1.1.1
    name: Old Name
"#,
        )
        .unwrap();
        reconcile(&pool, &config1).await.unwrap();

        let config2: KemuriConfig = serde_yaml::from_str(
            r#"
version: 1
targets:
  - id: t1
    address: 1.1.1.1
    name: New Name
"#,
        )
        .unwrap();
        reconcile(&pool, &config2).await.unwrap();

        let t1 = TargetRepo::get_by_target_id(&pool, "t1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(t1.name, "New Name");
    }

    #[tokio::test]
    async fn reconcile_rejects_probe_type_change() {
        let pool = setup_pool().await;

        let config1: KemuriConfig = serde_yaml::from_str(
            r#"
version: 1
profiles:
  - kind: icmp
    id: p1
targets:
  - id: t1
    address: 1.1.1.1
    checks:
      - id: c1
        profile: p1
"#,
        )
        .unwrap();
        reconcile(&pool, &config1).await.unwrap();

        let config2: KemuriConfig = serde_yaml::from_str(
            r#"
version: 1
profiles:
  - kind: http
    id: p1
    url: http://example.com
targets:
  - id: t1
    address: 1.1.1.1
    checks:
      - id: c1
        profile: p1
"#,
        )
        .unwrap();
        let result = reconcile(&pool, &config2).await;
        assert!(result.is_err());
        let probe_type: String =
            sqlx::query_scalar("SELECT probe_type FROM checks WHERE check_id = 'c1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let event_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM config_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(probe_type, "icmp");
        assert_eq!(event_count, 1);
    }

    #[tokio::test]
    async fn reconcile_resolves_alert_for_removed_check_in_same_transaction() {
        let pool = setup_pool().await;
        let config: KemuriConfig = serde_yaml::from_str(
            r#"
version: 1
profiles:
  - kind: icmp
    id: p1
targets:
  - id: t1
    address: 127.0.0.1
    checks:
      - id: c1
        profile: p1
"#,
        )
        .unwrap();
        reconcile(&pool, &config).await.unwrap();
        let check_id: i64 =
            sqlx::query_scalar("SELECT internal_id FROM checks WHERE check_id = 'c1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let observer_id: i64 =
            sqlx::query_scalar("SELECT internal_id FROM observers WHERE observer_id = 'local'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO alert_states
             (rule_id, check_internal_id, observer_internal_id, state)
             VALUES ('removed-rule', ?, ?, 'firing')",
        )
        .bind(check_id)
        .bind(observer_id)
        .execute(&pool)
        .await
        .unwrap();

        let disabled: KemuriConfig = serde_yaml::from_str(
            r#"
version: 1
profiles:
  - kind: icmp
    id: p1
targets:
  - id: t1
    address: 127.0.0.1
    checks:
      - id: c1
        profile: p1
        enabled: false
"#,
        )
        .unwrap();
        reconcile_with_event(&pool, &disabled, "reload")
            .await
            .unwrap();

        let state: String =
            sqlx::query_scalar("SELECT state FROM alert_states WHERE rule_id = 'removed-rule'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let reason: String =
            sqlx::query_scalar("SELECT reason FROM alert_events WHERE rule_id = 'removed-rule'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(state, "normal");
        assert_eq!(reason, "config_removed");
    }
}
