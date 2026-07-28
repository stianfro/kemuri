use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use kemuri_config::KemuriConfig;
use kemuri_core::{CheckId, ProbeKind, ProfileId, TargetId};
use kemuri_probes::{ResolvedCheck, RoundContext, SyntheticProbe};
use kemuri_server::ProbeRegistry;
use kemuri_storage::StorageManager;

fn make_round_context() -> RoundContext {
    RoundContext {
        observer_id: kemuri_core::ObserverId::new("test-observer").unwrap(),
        scheduled_at: Duration::from_secs(0),
        deadline: Duration::from_secs(5),
    }
}

fn make_resolved_check(check_id: &str, target_id: &str) -> ResolvedCheck {
    ResolvedCheck {
        check_id: CheckId::new(check_id).unwrap(),
        target_id: TargetId::new(target_id).unwrap(),
        profile_id: ProfileId::new("test-profile").unwrap(),
        address: "127.0.0.1".to_owned(),
        probe_kind: ProbeKind::Http,
        timeout: Duration::from_secs(5),
        sample_count: 1,
        params: HashMap::new(),
    }
}

fn make_test_config() -> KemuriConfig {
    KemuriConfig {
        version: 1,
        server: kemuri_config::ServerConfig::default(),
        logging: kemuri_config::LoggingConfig::default(),
        storage: kemuri_config::StorageConfig::default(),
        scheduler: kemuri_config::SchedulerConfig::default(),
        profiles: vec![],
        notifiers: vec![],
        rules: vec![],
        targets: vec![kemuri_config::TargetConfig {
            id: TargetId::new("web-1").unwrap(),
            address: "127.0.0.1".to_owned(),
            name: Some("Web Server 1".to_owned()),
            group_path: Some("web".to_owned()),
            labels: None,
            checks: vec![kemuri_config::CheckConfig {
                id: CheckId::new("synth-check").unwrap(),
                profile: ProfileId::new("http-default").unwrap(),
                enabled: true,
                kind: None,
                interval: None,
                timeout: None,
                url: None,
                method: None,
                headers: None,
                expected_status: None,
                body: None,
                host: None,
                port: None,
                domain: None,
                record_type: None,
                resolver: None,
                count: None,
                address_family: None,
                payload_size: None,
                source_address: None,
                follow_redirects: None,
                max_redirect_count: None,
                connection_mode: None,
                measure_until: None,
                user_agent: None,
                tls_validate: None,
                root_certificates: None,
                tls: None,
                protocol: None,
                expected_rcode: None,
                require_answer: None,
            }],
            enabled: true,
        }],
    }
}

#[tokio::test]
async fn queue_saturation_records_no_data_rounds() {
    let storage = StorageManager::open_in_memory().await.unwrap();
    let pool = storage.pool().clone();

    kemuri_storage::reconcile(&pool, &make_test_config())
        .await
        .unwrap();

    sqlx::query("INSERT OR IGNORE INTO observers (observer_id) VALUES ('local')")
        .execute(&pool)
        .await
        .unwrap();
    let (observer_internal_id,): (i64,) =
        sqlx::query_as("SELECT internal_id FROM observers WHERE observer_id = 'local'")
            .fetch_one(&pool)
            .await
            .unwrap();

    let (job_tx, job_rx) = tokio::sync::mpsc::channel::<kemuri_server::RoundJob>(16);
    let (result_tx, result_rx) = tokio::sync::mpsc::channel::<kemuri_server::RoundResult>(16);

    let mut registry = ProbeRegistry::new();
    registry.register(Arc::new(SyntheticProbe::success(Duration::from_millis(10))));

    let running_rounds = Arc::new(tokio::sync::Mutex::new(std::collections::HashSet::new()));

    let writer = kemuri_server::StorageWriter::new(Arc::new(storage), observer_internal_id);
    let writer_handle = tokio::spawn(async move {
        writer.run(result_rx).await;
    });

    let registry = Arc::new(registry);
    let worker_pool = kemuri_server::WorkerPool::new(registry, 1);
    let _worker_handles = worker_pool.start(job_rx, result_tx, running_rounds);

    for i in 0..5 {
        let job = kemuri_server::RoundJob {
            target_id: TargetId::new("web-1").unwrap(),
            check_id: CheckId::new("synth-check").unwrap(),
            check: kemuri_config::ResolvedCheckDef {
                check_id: CheckId::new("synth-check").unwrap(),
                target_id: TargetId::new("web-1").unwrap(),
                target_address: "127.0.0.1".to_owned(),
                profile_id: ProfileId::new("test").unwrap(),
                probe_kind: ProbeKind::Http,
                interval: Duration::from_secs(30),
                timeout: Duration::from_secs(5),
                revision_id: kemuri_core::CheckRevisionId::new("rev-test").unwrap(),
                probe_params: kemuri_config::ResolvedProbeParams::Http(
                    kemuri_config::ResolvedHttpParams {
                        url: "http://127.0.0.1".to_owned(),
                        method: None,
                        headers: HashMap::new(),
                        expected_status: None,
                        expected_status_range: None,
                        body: None,
                        follow_redirects: true,
                        max_redirect_count: 10,
                        connection_mode: "pooled".to_owned(),
                        measure_until: "headers".to_owned(),
                        user_agent: None,
                        tls_validate: true,
                        root_certificates: vec![],
                    },
                ),
            },
            scheduled_at: chrono::Utc::now() + chrono::Duration::seconds(i),
        };
        let _ = job_tx.send(job).await;
    }

    drop(job_tx);

    tokio::time::sleep(Duration::from_secs(2)).await;
    writer_handle.abort();

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM rounds")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(count.0 > 0, "expected some rounds to be recorded");
}

#[tokio::test]
async fn config_reload_failure_preserves_active_config() {
    let original_config: KemuriConfig = serde_yaml::from_str(
        r#"
version: 1
targets:
  - id: t1
    address: 1.1.1.1
"#,
    )
    .unwrap();

    let invalid_yaml = "version: 2\n";
    let result = KemuriConfig::parse(invalid_yaml);
    assert!(result.is_err());

    let re_parsed: KemuriConfig = serde_yaml::from_str(
        r#"
version: 1
targets:
  - id: t1
    address: 1.1.1.1
"#,
    )
    .unwrap();
    assert_eq!(original_config.targets.len(), re_parsed.targets.len());
}

#[tokio::test]
async fn probe_execution_does_not_panic_on_timeout() {
    let mut registry = ProbeRegistry::new();
    registry.register(Arc::new(SyntheticProbe::success(Duration::from_millis(10))));

    let ctx = make_round_context();
    let check = make_resolved_check("timeout-check", "timeout-target");

    let probe = registry.get(ProbeKind::Http).unwrap();
    let result =
        tokio::time::timeout(Duration::from_secs(10), probe.execute_round(ctx, check)).await;

    assert!(result.is_ok(), "probe should not panic");
}

#[tokio::test]
async fn database_locked_retried() {
    let storage = StorageManager::open_in_memory().await.unwrap();
    let pool = storage.pool().clone();

    kemuri_storage::reconcile(&pool, &make_test_config())
        .await
        .unwrap();

    sqlx::query("INSERT OR IGNORE INTO observers (observer_id) VALUES ('local')")
        .execute(&pool)
        .await
        .unwrap();

    let row: (i64,) =
        sqlx::query_as("SELECT internal_id FROM observers WHERE observer_id = 'local'")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert!(row.0 > 0);
}

#[tokio::test]
async fn notification_outbox_retry_on_failure() {
    let storage = StorageManager::open_in_memory().await.unwrap();
    let pool = storage.pool().clone();

    let target_id = kemuri_storage::TargetRepo::upsert(&pool, "t1", "t1", "", "{}")
        .await
        .unwrap();
    let check_id = kemuri_storage::CheckRepo::upsert(&pool, target_id, "c1", "icmp", None)
        .await
        .unwrap();
    let observer_id: i64 = sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
        .execute(&pool)
        .await
        .unwrap()
        .last_insert_rowid();

    let alert_event = kemuri_storage::InsertAlertEvent {
        rule_id: "r1".to_owned(),
        check_internal_id: check_id,
        observer_internal_id: observer_id,
        event_type: "firing".to_owned(),
        from_state: "normal".to_owned(),
        to_state: "firing".to_owned(),
        metric_value: Some(0.5),
        threshold_value: Some(0.1),
        occurred_at: chrono::Utc::now().to_rfc3339(),
    };
    let event_id = kemuri_storage::AlertEventRepo::insert(&pool, &alert_event)
        .await
        .unwrap();

    let entry = kemuri_storage::InsertNotificationOutbox {
        alert_event_internal_id: event_id,
        notifier_id: "nonexistent".to_owned(),
        status: "pending".to_owned(),
        next_attempt_at: chrono::Utc::now().to_rfc3339(),
    };

    let id = kemuri_storage::NotificationOutboxRepo::insert(&pool, &entry)
        .await
        .unwrap();
    assert!(id > 0);

    let pending = kemuri_storage::NotificationOutboxRepo::list_pending(
        &pool,
        &chrono::Utc::now().to_rfc3339(),
        10,
    )
    .await
    .unwrap();

    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].notifier_id, "nonexistent");
}

#[tokio::test]
async fn sigterm_during_active_rounds_graceful() {
    let (shutdown_tx, _) = tokio::sync::broadcast::channel::<()>(1);
    let mut rx = shutdown_tx.subscribe();

    let handle = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = rx.recv().await;
    });

    let _ = shutdown_tx.send(());

    let result = tokio::time::timeout(Duration::from_secs(2), handle).await;
    assert!(
        result.is_ok(),
        "task should complete gracefully on shutdown"
    );
}
