use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct SystemEvent {
    pub event_type: String,
    pub data: serde_json::Value,
}

impl SystemEvent {
    pub fn round_completed(target_id: &str, check_id: &str) -> Self {
        Self {
            event_type: "round.completed".to_owned(),
            data: serde_json::json!({
                "target_id": target_id,
                "check_id": check_id,
            }),
        }
    }

    pub fn check_state_changed(
        target_id: &str,
        check_id: &str,
        old_state: &str,
        new_state: &str,
    ) -> Self {
        Self {
            event_type: "check.state_changed".to_owned(),
            data: serde_json::json!({
                "target_id": target_id,
                "check_id": check_id,
                "old_state": old_state,
                "new_state": new_state,
            }),
        }
    }

    pub fn alert_firing(rule_id: &str, target_id: &str, check_id: &str) -> Self {
        Self {
            event_type: "alert.firing".to_owned(),
            data: serde_json::json!({
                "rule_id": rule_id,
                "target_id": target_id,
                "check_id": check_id,
            }),
        }
    }

    pub fn alert_resolved(rule_id: &str, target_id: &str, check_id: &str) -> Self {
        Self {
            event_type: "alert.resolved".to_owned(),
            data: serde_json::json!({
                "rule_id": rule_id,
                "target_id": target_id,
                "check_id": check_id,
            }),
        }
    }

    pub fn config_reloaded(generation: &str, result: &str) -> Self {
        Self {
            event_type: "config.reloaded".to_owned(),
            data: serde_json::json!({
                "generation": generation,
                "result": result,
            }),
        }
    }

    pub fn system_status_changed(status: &str) -> Self {
        Self {
            event_type: "system.status_changed".to_owned(),
            data: serde_json::json!({
                "status": status,
            }),
        }
    }

    pub fn to_sse_event(&self) -> axum::response::sse::Event {
        let data = serde_json::to_string(&self.data).unwrap_or_default();
        axum::response::sse::Event::default()
            .event(&self.event_type)
            .data(data)
    }
}
