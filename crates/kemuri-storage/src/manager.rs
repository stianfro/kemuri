use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::sqlite::SqliteJournalMode;
use sqlx::sqlite::SqlitePoolOptions;
use tokio::sync::mpsc;

use crate::repos::{ConfigEventRepo, InsertConfigEvent, InsertRound, RoundInsertError, RoundRepo};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("channel closed")]
    ChannelClosed,
    #[error("duplicate round")]
    DuplicateRound,
}

pub enum WriteOp {
    InsertRound {
        round: Box<InsertRound>,
        result_tx: mpsc::Sender<Result<i64, RoundInsertError>>,
    },
    InsertConfigEvent {
        event: Box<InsertConfigEvent>,
        result_tx: mpsc::Sender<Result<i64, sqlx::Error>>,
    },
}

pub struct StorageManager {
    pool: SqlitePool,
    write_tx: mpsc::Sender<WriteOp>,
    write_handle: Option<tokio::task::JoinHandle<()>>,
}

impl StorageManager {
    pub async fn open(db_path: &str) -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::from_str(&format!("sqlite://{}?mode=rwc", db_path))?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));

        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .acquire_timeout(Duration::from_secs(5))
            .connect_with(options)
            .await?;

        Self::run_integrity_check(&pool).await?;

        sqlx::migrate!().run(&pool).await?;

        let (write_tx, write_rx) = mpsc::channel::<WriteOp>(256);

        let writer_pool = pool.clone();
        let write_handle = tokio::spawn(async move {
            Self::writer_task(write_rx, writer_pool).await;
        });

        Ok(Self {
            pool,
            write_tx,
            write_handle: Some(write_handle),
        })
    }

    pub async fn open_in_memory() -> Result<Self, StorageError> {
        let options = SqliteConnectOptions::from_str("sqlite::memory:")?.foreign_keys(true);

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;

        sqlx::migrate!().run(&pool).await?;

        let (write_tx, write_rx) = mpsc::channel::<WriteOp>(256);

        let writer_pool = pool.clone();
        let write_handle = tokio::spawn(async move {
            Self::writer_task(write_rx, writer_pool).await;
        });

        Ok(Self {
            pool,
            write_tx,
            write_handle: Some(write_handle),
        })
    }

    async fn run_integrity_check(pool: &SqlitePool) -> Result<(), StorageError> {
        let row: (String,) = sqlx::query_as("PRAGMA integrity_check")
            .fetch_one(pool)
            .await?;
        if row.0 != "ok" {
            tracing::warn!("database integrity check: {}", row.0);
        }
        Ok(())
    }

    async fn writer_task(mut rx: mpsc::Receiver<WriteOp>, pool: SqlitePool) {
        while let Some(op) = rx.recv().await {
            match op {
                WriteOp::InsertRound { round, result_tx } => {
                    let result = RoundRepo::insert(&pool, &round).await;
                    let _ = result_tx.send(result).await;
                }
                WriteOp::InsertConfigEvent { event, result_tx } => {
                    let result = ConfigEventRepo::insert(&pool, &event).await;
                    let _ = result_tx.send(result).await;
                }
            }
        }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn write_round(&self, round: InsertRound) -> Result<i64, StorageError> {
        let (result_tx, mut result_rx) = mpsc::channel(1);
        self.write_tx
            .send(WriteOp::InsertRound {
                round: Box::new(round),
                result_tx,
            })
            .await
            .map_err(|_| StorageError::ChannelClosed)?;
        result_rx
            .recv()
            .await
            .ok_or(StorageError::ChannelClosed)?
            .map_err(|e| match e {
                RoundInsertError::Duplicate => StorageError::DuplicateRound,
                RoundInsertError::Db(e) => StorageError::Db(e),
            })
    }

    pub async fn write_config_event(&self, event: InsertConfigEvent) -> Result<i64, StorageError> {
        let (result_tx, mut result_rx) = mpsc::channel(1);
        self.write_tx
            .send(WriteOp::InsertConfigEvent {
                event: Box::new(event),
                result_tx,
            })
            .await
            .map_err(|_| StorageError::ChannelClosed)?;
        result_rx
            .recv()
            .await
            .ok_or(StorageError::ChannelClosed)?
            .map_err(StorageError::Db)
    }

    pub async fn shutdown(&mut self) {
        drop(self.write_tx.clone());
        if let Some(handle) = self.write_handle.take() {
            handle.abort();
        }
    }
}

impl Drop for StorageManager {
    fn drop(&mut self) {
        if let Some(handle) = self.write_handle.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repos::{CheckCurrentStateRepo, CheckRepo, TargetRepo, UpsertCheckCurrentState};

    #[tokio::test]
    async fn fresh_migration() {
        let mgr = StorageManager::open_in_memory().await.unwrap();
        let row: (i32,) = sqlx::query_as("SELECT COUNT(*) FROM targets")
            .fetch_one(mgr.pool())
            .await
            .unwrap();
        assert_eq!(row.0, 0);
    }

    #[tokio::test]
    async fn write_and_read_round() {
        let mgr = StorageManager::open_in_memory().await.unwrap();
        let pool = mgr.pool();

        let target_id = TargetRepo::upsert(pool, "t1", "t1", "", "{}")
            .await
            .unwrap();
        let check_id = CheckRepo::upsert(pool, target_id, "c1", "icmp", None)
            .await
            .unwrap();

        let observer_id: i64 = sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let round = InsertRound {
            check_internal_id: check_id,
            observer_internal_id: observer_id,
            scheduled_at: "2024-01-01T00:00:00Z".to_owned(),
            started_at: Some("2024-01-01T00:00:00.001Z".to_owned()),
            finished_at: Some("2024-01-01T00:00:00.002Z".to_owned()),
            execution_status: "complete".to_owned(),
            stop_reason: None,
            configured_samples: 3,
            attempted_samples: 3,
            latency_bearing_samples: 3,
            healthy_samples: 3,
            unhealthy_samples: 0,
            measurement_loss_samples: 0,
            min_latency_ns: Some(1_000_000),
            median_latency_ns: Some(1_500_000),
            max_latency_ns: Some(2_000_000),
            sample_blob: None,
            outcome_summary: Some("3/3 healthy".to_owned()),
            config_generation: None,
            check_revision_id: None,
        };

        let id = mgr.write_round(round).await.unwrap();
        assert!(id > 0);

        let latest = RoundRepo::get_latest(pool, check_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(latest.min_latency_ns, Some(1_000_000));
    }

    #[tokio::test]
    async fn duplicate_round_rejected() {
        let mgr = StorageManager::open_in_memory().await.unwrap();
        let pool = mgr.pool();

        let target_id = TargetRepo::upsert(pool, "t1", "t1", "", "{}")
            .await
            .unwrap();
        let check_id = CheckRepo::upsert(pool, target_id, "c1", "icmp", None)
            .await
            .unwrap();

        let observer_id: i64 = sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let round = InsertRound {
            check_internal_id: check_id,
            observer_internal_id: observer_id,
            scheduled_at: "2024-01-01T00:00:00Z".to_owned(),
            started_at: None,
            finished_at: None,
            execution_status: "complete".to_owned(),
            stop_reason: None,
            configured_samples: 3,
            attempted_samples: 3,
            latency_bearing_samples: 3,
            healthy_samples: 3,
            unhealthy_samples: 0,
            measurement_loss_samples: 0,
            min_latency_ns: None,
            median_latency_ns: None,
            max_latency_ns: None,
            sample_blob: None,
            outcome_summary: None,
            config_generation: None,
            check_revision_id: None,
        };

        mgr.write_round(round.clone()).await.unwrap();
        let result = mgr.write_round(round).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn current_state_upsert() {
        let mgr = StorageManager::open_in_memory().await.unwrap();
        let pool = mgr.pool();

        let target_id = TargetRepo::upsert(pool, "t1", "t1", "", "{}")
            .await
            .unwrap();
        let check_id = CheckRepo::upsert(pool, target_id, "c1", "icmp", None)
            .await
            .unwrap();

        let observer_id: i64 = sqlx::query("INSERT INTO observers (observer_id) VALUES ('obs1')")
            .execute(pool)
            .await
            .unwrap()
            .last_insert_rowid();

        let state = UpsertCheckCurrentState {
            check_internal_id: check_id,
            observer_internal_id: observer_id,
            state: "healthy".to_owned(),
            last_round_at: Some("2024-01-01T00:00:00Z".to_owned()),
            last_latency_ns: Some(1_500_000),
            last_measurement_loss_ratio: Some(0.0),
            last_health_failure_ratio: Some(0.0),
        };

        CheckCurrentStateRepo::upsert(pool, &state).await.unwrap();

        let row = CheckCurrentStateRepo::get(pool, check_id, observer_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "healthy");
        assert_eq!(row.last_latency_ns, Some(1_500_000));

        let state2 = UpsertCheckCurrentState {
            state: "degraded".to_owned(),
            ..state
        };
        CheckCurrentStateRepo::upsert(pool, &state2).await.unwrap();

        let row = CheckCurrentStateRepo::get(pool, check_id, observer_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "degraded");
    }
}
