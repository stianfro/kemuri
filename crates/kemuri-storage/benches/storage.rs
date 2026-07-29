use criterion::{Criterion, criterion_group, criterion_main};
use kemuri_storage::{RollupRepo, RoundRepo, StorageManager};

fn seeded_runtime() -> (tokio::runtime::Runtime, StorageManager) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let storage = runtime.block_on(async {
        let storage = StorageManager::open_in_memory().await.unwrap();
        let pool = storage.pool();
        sqlx::query("INSERT INTO targets (internal_id, target_id, name) VALUES (1, 't', 't')")
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO checks (internal_id, target_internal_id, check_id, probe_type)
             VALUES (1, 1, 'c', 'http')",
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO observers (internal_id, observer_id) VALUES (1, 'local')")
            .execute(pool)
            .await
            .unwrap();
        for index in 0..1_000 {
            let timestamp = format!("2025-01-01T00:{:02}:{:02}Z", index / 60, index % 60);
            sqlx::query(
                "INSERT INTO rounds
                 (check_internal_id, observer_internal_id, scheduled_at, execution_status,
                  configured_samples, attempted_samples, latency_bearing_samples, healthy_samples)
                 VALUES (1, 1, ?, 'complete', 1, 1, 1, 1)",
            )
            .bind(&timestamp)
            .execute(pool)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO rollups
                 (check_internal_id, observer_internal_id, resolution_seconds, bucket_start,
                  scheduled_rounds, completed_rounds)
                 VALUES (1, 1, 300, ?, 1, 1)",
            )
            .bind(timestamp)
            .execute(pool)
            .await
            .unwrap();
        }
        storage
    });
    (runtime, storage)
}

fn storage_queries(c: &mut Criterion) {
    let (runtime, storage) = seeded_runtime();
    c.bench_function("series_query_1000_rounds", |bencher| {
        bencher.iter(|| {
            runtime
                .block_on(RoundRepo::query_by_check_range_with_observer(
                    storage.pool(),
                    1,
                    1,
                    "2025-01-01T00:00:00Z",
                    "2025-01-01T99:00:00Z",
                ))
                .unwrap()
        })
    });
    c.bench_function("rollup_query_1000_buckets", |bencher| {
        bencher.iter(|| {
            runtime
                .block_on(RollupRepo::query_by_check_and_range(
                    storage.pool(),
                    1,
                    1,
                    300,
                    "2025-01-01T00:00:00Z",
                    "2025-01-01T99:00:00Z",
                ))
                .unwrap()
        })
    });
}

criterion_group!(benches, storage_queries);
criterion_main!(benches);
