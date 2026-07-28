#![allow(dead_code)]
use sqlx::FromRow;

mod alert_state_repo;
mod check_assignment_repo;
mod check_current_state_repo;
mod check_repo;
mod config_event_repo;
mod notification_outbox_repo;
mod rollup_repo;
mod round_repo;
mod target_repo;

#[allow(unused_imports)]
pub use alert_state_repo::{
    AlertEventRepo, AlertEventRow, AlertStateRepo, AlertStateRow, InsertAlertEvent,
    UpsertAlertState,
};
#[allow(unused_imports)]
pub use check_assignment_repo::CheckAssignmentRepo;
#[allow(unused_imports)]
pub use check_current_state_repo::CheckCurrentStateRepo;
pub use check_repo::{CheckRepo, CheckWithState};
pub use config_event_repo::ConfigEventRepo;
pub use notification_outbox_repo::{
    InsertNotificationOutbox, NotificationOutboxRepo, NotificationOutboxRow,
};
pub use rollup_repo::RollupRepo;
pub use round_repo::{RoundInsertError, RoundRepo};
pub use target_repo::{TargetRepo, TargetWithState};

#[derive(Debug, Clone, FromRow)]
pub struct TargetRow {
    pub internal_id: i64,
    pub target_id: String,
    pub name: String,
    pub group_path: String,
    pub labels: String,
    pub active: bool,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct CheckRow {
    pub internal_id: i64,
    pub target_internal_id: i64,
    pub check_id: String,
    pub probe_type: String,
    pub active: bool,
    pub current_revision_id: Option<String>,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct RoundRow {
    pub internal_id: i64,
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub scheduled_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub execution_status: String,
    pub stop_reason: Option<String>,
    pub configured_samples: i32,
    pub attempted_samples: i32,
    pub latency_bearing_samples: i32,
    pub healthy_samples: i32,
    pub unhealthy_samples: i32,
    pub measurement_loss_samples: i32,
    pub min_latency_ns: Option<i64>,
    pub median_latency_ns: Option<i64>,
    pub max_latency_ns: Option<i64>,
    pub sample_blob: Option<Vec<u8>>,
    pub outcome_summary: Option<String>,
    pub config_generation: Option<String>,
    pub check_revision_id: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone)]
pub struct InsertRound {
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub scheduled_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
    pub execution_status: String,
    pub stop_reason: Option<String>,
    pub configured_samples: i32,
    pub attempted_samples: i32,
    pub latency_bearing_samples: i32,
    pub healthy_samples: i32,
    pub unhealthy_samples: i32,
    pub measurement_loss_samples: i32,
    pub min_latency_ns: Option<i64>,
    pub median_latency_ns: Option<i64>,
    pub max_latency_ns: Option<i64>,
    pub sample_blob: Option<Vec<u8>>,
    pub outcome_summary: Option<String>,
    pub config_generation: Option<String>,
    pub check_revision_id: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct CheckCurrentStateRow {
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub state: String,
    pub last_round_at: Option<String>,
    pub last_latency_ns: Option<i64>,
    pub last_measurement_loss_ratio: Option<f64>,
    pub last_health_failure_ratio: Option<f64>,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct UpsertCheckCurrentState {
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub state: String,
    pub last_round_at: Option<String>,
    pub last_latency_ns: Option<i64>,
    pub last_measurement_loss_ratio: Option<f64>,
    pub last_health_failure_ratio: Option<f64>,
}

#[derive(Debug, Clone, FromRow)]
pub struct ConfigEventRow {
    pub internal_id: i64,
    pub generation_hash: String,
    pub event_type: String,
    pub summary: Option<String>,
    pub occurred_at: String,
}

#[derive(Debug, Clone)]
pub struct InsertConfigEvent {
    pub generation_hash: String,
    pub event_type: String,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, FromRow)]
pub struct CheckAssignmentRow {
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub active: bool,
    pub assigned_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct ObserverRow {
    pub internal_id: i64,
    pub observer_id: String,
    pub status: String,
    pub first_seen_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub struct RollupRow {
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub resolution_seconds: i64,
    pub bucket_start: String,
    pub scheduled_rounds: i64,
    pub completed_rounds: i64,
    pub partial_rounds: i64,
    pub configured_sample_slots: i64,
    pub attempted_samples: i64,
    pub latency_bearing_samples: i64,
    pub healthy_samples: i64,
    pub unhealthy_samples: i64,
    pub measurement_loss_samples: i64,
    pub outcome_counts: String,
    pub min_latency_ns: Option<i64>,
    pub max_latency_ns: Option<i64>,
    pub sum_latency_ns: i64,
    pub histogram_version: i32,
    pub histogram_blob: Option<Vec<u8>>,
    pub no_data_counts: String,
}

#[derive(Debug, Clone)]
pub struct InsertRollup {
    pub check_internal_id: i64,
    pub observer_internal_id: i64,
    pub resolution_seconds: i64,
    pub bucket_start: String,
    pub scheduled_rounds: i64,
    pub completed_rounds: i64,
    pub partial_rounds: i64,
    pub configured_sample_slots: i64,
    pub attempted_samples: i64,
    pub latency_bearing_samples: i64,
    pub healthy_samples: i64,
    pub unhealthy_samples: i64,
    pub measurement_loss_samples: i64,
    pub outcome_counts: String,
    pub min_latency_ns: Option<i64>,
    pub max_latency_ns: Option<i64>,
    pub sum_latency_ns: i64,
    pub histogram_version: i32,
    pub histogram_blob: Option<Vec<u8>>,
    pub no_data_counts: String,
}
