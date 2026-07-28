use std::collections::HashMap;
use std::time::Duration;

use async_trait::async_trait;
use kemuri_core::NotifierId;

use super::{NotificationError, NotificationPayload, NotificationResult, Notifier, resolve_secret};

pub struct WebhookNotifier {
    id: NotifierId,
    url: String,
    headers: HashMap<String, String>,
    client: reqwest::Client,
}

impl WebhookNotifier {
    pub fn new(
        id: NotifierId,
        url: String,
        headers: HashMap<String, String>,
        timeout: Duration,
    ) -> Result<Self, NotificationError> {
        let client = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| {
                NotificationError::Config(format!("failed to create HTTP client: {}", e))
            })?;
        Ok(Self {
            id,
            url,
            headers,
            client,
        })
    }

    pub fn from_config(
        params: &kemuri_config::WebhookNotifierParams,
    ) -> Result<Self, NotificationError> {
        let url = resolve_secret(&params.url)?;
        let mut headers = HashMap::new();
        if let Some(ref header_refs) = params.headers {
            for (key, secret) in header_refs {
                let value = resolve_secret(secret)?;
                headers.insert(key.clone(), value);
            }
        }
        let timeout =
            kemuri_core::parse_duration(&params.timeout).unwrap_or(Duration::from_secs(10));
        Self::new(params.id.clone(), url, headers, timeout)
    }
}

#[async_trait]
impl Notifier for WebhookNotifier {
    fn kind(&self) -> &str {
        "webhook"
    }

    fn id(&self) -> &NotifierId {
        &self.id
    }

    async fn send(&self, notification: NotificationPayload) -> NotificationResult {
        let mut body = serde_json::json!({
            "schema_version": "1",
            "event_id": notification.event_id,
            "event_type": match notification.event_type {
                kemuri_core::AlertEventKind::Firing => "firing",
                kemuri_core::AlertEventKind::Resolved => "resolved",
            },
            "rule_id": notification.rule_id.to_string(),
            "target_id": notification.target_id.to_string(),
            "target_name": notification.target_name,
            "check_id": notification.check_id.to_string(),
            "observer_id": notification.observer_id.to_string(),
            "probe_type": notification.probe_type.to_string(),
            "current_value": notification.current_value,
            "threshold": notification.threshold,
            "state_start_time": notification.state_start_time.to_rfc3339(),
            "event_time": notification.event_time.to_rfc3339(),
            "labels": notification.labels,
            "summary": notification.summary,
        });

        if let Some(ref url) = notification.kemuri_url {
            body["kemuri_url"] = serde_json::Value::String(url.clone());
        }

        let mut request = self
            .client
            .post(&self.url)
            .header("Content-Type", "application/json")
            .json(&body);

        for (key, value) in &self.headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request
            .send()
            .await
            .map_err(|e| NotificationError::Delivery(format!("webhook request failed: {}", e)))?;

        let status = response.status();
        if status.as_u16() >= 200 && status.as_u16() < 300 {
            Ok(())
        } else {
            let body_text = response.text().await.unwrap_or_default();
            let truncated = if body_text.len() > 1024 {
                &body_text[..1024]
            } else {
                &body_text
            };
            Err(NotificationError::Delivery(format!(
                "webhook returned status {}: {}",
                status, truncated
            )))
        }
    }
}
