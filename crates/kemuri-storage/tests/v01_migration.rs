use std::path::PathBuf;

use kemuri_storage::StorageManager;

#[tokio::test]
async fn released_v01_database_migrates_without_data_loss() {
    let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/v0.1.sqlite");
    let database = std::env::temp_dir().join(format!(
        "kemuri-v01-migration-{}-{}.sqlite",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::copy(fixture, &database).unwrap();

    let storage = StorageManager::open(database.to_str().unwrap())
        .await
        .unwrap();
    let pool = storage.pool();
    let migration_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _sqlx_migrations")
        .fetch_one(pool)
        .await
        .unwrap();
    let round_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rounds")
        .fetch_one(pool)
        .await
        .unwrap();
    let revision_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM check_revisions")
        .fetch_one(pool)
        .await
        .unwrap();
    let alert_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM alert_events")
        .fetch_one(pool)
        .await
        .unwrap();
    let notification_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notification_outbox")
        .fetch_one(pool)
        .await
        .unwrap();
    let reason_column: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('alert_events') WHERE name = 'reason'",
    )
    .fetch_one(pool)
    .await
    .unwrap();

    assert_eq!(migration_count, 2);
    assert_eq!(round_count, 1);
    assert_eq!(revision_count, 1);
    assert_eq!(alert_count, 1);
    assert_eq!(notification_count, 1);
    assert_eq!(reason_column, 1);

    drop(storage);
    std::fs::remove_file(database).unwrap();
}
