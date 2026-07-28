use std::collections::HashSet;

use sqlx::SqlitePool;

use kemuri_config::KemuriConfig;

use crate::repos::{CheckRepo, TargetRepo};

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
}

pub async fn reconcile(
    pool: &SqlitePool,
    config: &KemuriConfig,
) -> Result<(), ReconciliationError> {
    let active_targets = TargetRepo::list_active(pool).await?;
    let active_target_ids: HashSet<String> =
        active_targets.iter().map(|t| t.target_id.clone()).collect();

    let config_target_ids: HashSet<String> =
        config.targets.iter().map(|t| t.id.to_string()).collect();

    for removed_id in active_target_ids.difference(&config_target_ids) {
        TargetRepo::deactivate(pool, removed_id).await?;
    }

    for target in &config.targets {
        let target_id_str = target.id.to_string();
        let name = target.name.as_deref().unwrap_or(&target_id_str);
        let group_path = target.group_path.as_deref().unwrap_or("");
        let labels = target
            .labels
            .as_ref()
            .map(|m| serde_json::to_string(m).unwrap_or_default())
            .unwrap_or_default();

        let target_internal_id =
            TargetRepo::upsert(pool, &target_id_str, name, group_path, &labels).await?;

        let active_checks = CheckRepo::list_active_by_target(pool, target_internal_id).await?;
        let active_check_ids: HashSet<String> =
            active_checks.iter().map(|c| c.check_id.clone()).collect();

        let config_check_ids: HashSet<String> =
            target.checks.iter().map(|c| c.id.to_string()).collect();

        for existing_check in &active_checks {
            if let Some(config_check) = target
                .checks
                .iter()
                .find(|c| c.id.to_string() == existing_check.check_id)
            {
                let profile = config
                    .profiles
                    .iter()
                    .find(|p| p.id() == &config_check.profile);
                if let Some(profile) = profile {
                    let new_type = profile.kind().to_string();
                    if existing_check.probe_type != new_type {
                        return Err(ReconciliationError::ProbeTypeMismatch {
                            check_id: existing_check.check_id.clone(),
                            existing: existing_check.probe_type.clone(),
                            new: new_type,
                        });
                    }
                }
            }
        }

        for removed_check_id in active_check_ids.difference(&config_check_ids) {
            CheckRepo::deactivate(pool, target_internal_id, removed_check_id).await?;
        }

        for check in &target.checks {
            let check_id_str = check.id.to_string();
            let profile = config.profiles.iter().find(|p| p.id() == &check.profile);
            let probe_type = profile.map(|p| p.kind().to_string()).unwrap_or_default();
            let revision_id: Option<&str> = None;

            CheckRepo::upsert(
                pool,
                target_internal_id,
                &check_id_str,
                &probe_type,
                revision_id,
            )
            .await?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
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
    }
}
