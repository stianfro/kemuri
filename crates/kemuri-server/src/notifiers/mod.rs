pub mod smtp;
pub mod webhook;

use std::collections::HashMap;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use kemuri_core::{AlertEventKind, CheckId, NotifierId, ObserverId, ProbeKind, RuleId, TargetId};

pub use smtp::SmtpNotifier;
pub use webhook::WebhookNotifier;

#[derive(Debug, Clone)]
pub struct NotificationPayload {
    pub event_id: String,
    pub event_type: AlertEventKind,
    pub rule_id: RuleId,
    pub target_id: TargetId,
    pub target_name: String,
    pub check_id: CheckId,
    pub observer_id: ObserverId,
    pub probe_type: ProbeKind,
    pub current_value: f64,
    pub threshold: f64,
    pub state_start_time: DateTime<Utc>,
    pub event_time: DateTime<Utc>,
    pub kemuri_url: Option<String>,
    pub labels: HashMap<String, String>,
    pub summary: String,
}

#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    #[error("delivery failed: {0}")]
    Delivery(String),
    #[error("configuration error: {0}")]
    Config(String),
}

pub type NotificationResult = Result<(), NotificationError>;

#[async_trait]
pub trait Notifier: Send + Sync {
    fn kind(&self) -> &str;
    fn id(&self) -> &NotifierId;
    async fn send(&self, notification: NotificationPayload) -> NotificationResult;
}

pub fn resolve_secret(secret: &kemuri_config::SecretRef) -> Result<String, NotificationError> {
    match secret {
        kemuri_config::SecretRef::Literal(s) => Ok(s.clone()),
        kemuri_config::SecretRef::FromEnv { from_env } => std::env::var(from_env).map_err(|_| {
            NotificationError::Config(format!("environment variable not found: {}", from_env))
        }),
        kemuri_config::SecretRef::FromFile { from_file } => std::fs::read_to_string(from_file)
            .map(|s| s.trim().to_owned())
            .map_err(|e| {
                NotificationError::Config(format!(
                    "failed to read secret from {}: {}",
                    from_file, e
                ))
            }),
    }
}
