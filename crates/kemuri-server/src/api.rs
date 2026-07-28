use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};

use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub request_id: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.code.as_str() {
            "target_not_found" | "check_not_found" | "alert_not_found" => StatusCode::NOT_FOUND,
            "bad_request" | "invalid_params" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(self)).into_response()
    }
}

fn make_request_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

fn not_found(code: &str, message: &str) -> ApiError {
    ApiError {
        code: code.to_owned(),
        message: message.to_owned(),
        request_id: make_request_id(),
    }
}

fn bad_request(message: &str) -> ApiError {
    ApiError {
        code: "bad_request".to_owned(),
        message: message.to_owned(),
        request_id: make_request_id(),
    }
}

fn internal_error(e: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %e, "API internal error");
    ApiError {
        code: "internal_error".to_owned(),
        message: "An internal error occurred".to_owned(),
        request_id: make_request_id(),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlertsQuery {
    pub state: Option<String>,
    pub rule_id: Option<String>,
    pub target_id: Option<String>,
    pub check_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertStateResponse {
    pub internal_id: i64,
    pub rule_id: String,
    pub target_id: String,
    pub check_id: String,
    pub state: String,
    pub state_entered_at: String,
    pub first_condition_true_at: Option<String>,
    pub last_evaluated_at: Option<String>,
    pub last_notification_at: Option<String>,
    pub last_metric_value: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertsListResponse {
    pub alerts: Vec<AlertStateResponse>,
}

pub async fn list_alerts(
    State(state): State<AppState>,
    Query(query): Query<AlertsQuery>,
) -> Result<Json<AlertsListResponse>, ApiError> {
    let pool = state.pool.clone();

    let alerts = if let Some(ref rule_id) = query.rule_id {
        kemuri_storage::AlertStateRepo::list_by_rule_id(&pool, rule_id)
            .await
            .map_err(internal_error)?
    } else if let Some(ref state_filter) = query.state {
        let states: Vec<&str> = state_filter.split(',').collect();
        kemuri_storage::AlertStateRepo::list_by_state(&pool, &states)
            .await
            .map_err(internal_error)?
    } else {
        kemuri_storage::AlertStateRepo::list_all(&pool)
            .await
            .map_err(internal_error)?
    };

    let mut responses = Vec::new();
    for alert in &alerts {
        let check = kemuri_storage::CheckRepo::get_by_internal_id(&pool, alert.check_internal_id)
            .await
            .map_err(internal_error)?;

        let (target_id, check_id) = match check {
            Some(c) => {
                let target =
                    kemuri_storage::TargetRepo::get_by_internal_id(&pool, c.target_internal_id)
                        .await
                        .map_err(internal_error)?;
                (
                    target.map(|t| t.target_id.clone()).unwrap_or_default(),
                    c.check_id.clone(),
                )
            }
            None => (String::new(), String::new()),
        };

        if let Some(ref filter_target_id) = query.target_id
            && target_id != *filter_target_id
        {
            continue;
        }
        if let Some(ref filter_check_id) = query.check_id
            && check_id != *filter_check_id
        {
            continue;
        }

        responses.push(AlertStateResponse {
            internal_id: alert.internal_id,
            rule_id: alert.rule_id.clone(),
            target_id,
            check_id,
            state: alert.state.clone(),
            state_entered_at: alert.state_entered_at.clone(),
            first_condition_true_at: alert.first_condition_true_at.clone(),
            last_evaluated_at: alert.last_evaluated_at.clone(),
            last_notification_at: alert.last_notification_at.clone(),
            last_metric_value: alert.last_metric_value,
        });
    }

    Ok(Json(AlertsListResponse { alerts: responses }))
}

pub async fn get_alert(
    State(state): State<AppState>,
    Path(alert_id): Path<i64>,
) -> Result<Json<AlertStateResponse>, ApiError> {
    let pool = state.pool.clone();

    let alert = kemuri_storage::AlertStateRepo::get_by_internal_id(&pool, alert_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| {
            not_found(
                "alert_not_found",
                "The requested alert state does not exist",
            )
        })?;

    let check = kemuri_storage::CheckRepo::get_by_internal_id(&pool, alert.check_internal_id)
        .await
        .map_err(internal_error)?;

    let (target_id, check_id) = match check {
        Some(c) => {
            let target =
                kemuri_storage::TargetRepo::get_by_internal_id(&pool, c.target_internal_id)
                    .await
                    .map_err(internal_error)?;
            (
                target.map(|t| t.target_id.clone()).unwrap_or_default(),
                c.check_id.clone(),
            )
        }
        None => (String::new(), String::new()),
    };

    Ok(Json(AlertStateResponse {
        internal_id: alert.internal_id,
        rule_id: alert.rule_id.clone(),
        target_id,
        check_id,
        state: alert.state.clone(),
        state_entered_at: alert.state_entered_at.clone(),
        first_condition_true_at: alert.first_condition_true_at.clone(),
        last_evaluated_at: alert.last_evaluated_at.clone(),
        last_notification_at: alert.last_notification_at.clone(),
        last_metric_value: alert.last_metric_value,
    }))
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlertEventsQuery {
    pub rule_id: Option<String>,
    pub target_id: Option<String>,
    pub check_id: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertEventResponse {
    pub internal_id: i64,
    pub rule_id: String,
    pub target_id: String,
    pub check_id: String,
    pub event_type: String,
    pub from_state: String,
    pub to_state: String,
    pub metric_value: Option<f64>,
    pub threshold_value: Option<f64>,
    pub occurred_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AlertEventsResponse {
    pub events: Vec<AlertEventResponse>,
}

pub async fn list_alert_events(
    State(state): State<AppState>,
    Query(query): Query<AlertEventsQuery>,
) -> Result<Json<AlertEventsResponse>, ApiError> {
    let pool = state.pool.clone();
    let limit = query.limit.unwrap_or(100).min(500);

    let events = if let Some(ref rule_id) = query.rule_id {
        kemuri_storage::AlertEventRepo::list_by_rule(&pool, rule_id, limit)
            .await
            .map_err(internal_error)?
    } else if let Some(ref from) = query.from {
        let to = query.to.as_deref().unwrap_or("2099-01-01T00:00:00Z");
        let check_internal_id: i64 = if let Some(ref target_id) = query.target_id {
            let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, target_id)
                .await
                .map_err(internal_error)?;
            match target {
                Some(t) => {
                    if let Some(ref check_id) = query.check_id {
                        kemuri_storage::CheckRepo::get(&pool, t.internal_id, check_id)
                            .await
                            .map_err(internal_error)?
                            .map(|c| c.internal_id)
                            .unwrap_or(0)
                    } else {
                        0
                    }
                }
                None => 0,
            }
        } else {
            0
        };
        kemuri_storage::AlertEventRepo::list_by_check_range(
            &pool,
            check_internal_id,
            from,
            to,
            limit,
        )
        .await
        .map_err(internal_error)?
    } else {
        kemuri_storage::AlertEventRepo::list_recent(&pool, limit)
            .await
            .map_err(internal_error)?
    };

    let mut responses = Vec::new();
    for event in &events {
        let check = kemuri_storage::CheckRepo::get_by_internal_id(&pool, event.check_internal_id)
            .await
            .map_err(internal_error)?;

        let (target_id, check_id) = match check {
            Some(c) => {
                let target =
                    kemuri_storage::TargetRepo::get_by_internal_id(&pool, c.target_internal_id)
                        .await
                        .map_err(internal_error)?;
                (
                    target.map(|t| t.target_id.clone()).unwrap_or_default(),
                    c.check_id.clone(),
                )
            }
            None => (String::new(), String::new()),
        };

        responses.push(AlertEventResponse {
            internal_id: event.internal_id,
            rule_id: event.rule_id.clone(),
            target_id,
            check_id,
            event_type: event.event_type.clone(),
            from_state: event.from_state.clone(),
            to_state: event.to_state.clone(),
            metric_value: event.metric_value,
            threshold_value: event.threshold_value,
            occurred_at: event.occurred_at.clone(),
        });
    }

    Ok(Json(AlertEventsResponse { events: responses }))
}

#[derive(Debug, Clone, Serialize)]
pub struct GroupResponse {
    pub group_path: String,
    pub targets: Vec<TargetSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetSummary {
    pub target_id: String,
    pub name: String,
    pub group_path: String,
    pub state: String,
    pub checks_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetsResponse {
    pub targets: Vec<TargetSummary>,
    pub groups: Vec<GroupResponse>,
}

pub async fn list_targets(
    State(state): State<AppState>,
) -> Result<Json<TargetsResponse>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;

    let rows = kemuri_storage::TargetRepo::list_with_state(&pool, observer_id)
        .await
        .map_err(|e| ApiError {
            code: "internal_error".to_owned(),
            message: e.to_string(),
            request_id: make_request_id(),
        })?;

    let mut target_map: std::collections::HashMap<String, Vec<TargetSummary>> =
        std::collections::HashMap::new();
    let mut all_targets: Vec<TargetSummary> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for row in &rows {
        if seen.insert(row.target_id.clone()) {
            let group = if row.group_path.is_empty() {
                "default".to_owned()
            } else {
                row.group_path.clone()
            };
            let state_str = row.state.as_deref().unwrap_or("no_data");
            let summary = TargetSummary {
                target_id: row.target_id.clone(),
                name: row.name.clone(),
                group_path: group.clone(),
                state: state_str.to_owned(),
                checks_count: 0,
            };
            target_map.entry(group).or_default().push(summary.clone());
            all_targets.push(summary);
        }
    }

    let groups = target_map
        .into_iter()
        .map(|(group_path, targets)| GroupResponse {
            group_path,
            targets,
        })
        .collect();

    Ok(Json(TargetsResponse {
        targets: all_targets,
        groups,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckSummary {
    pub check_id: String,
    pub probe_type: String,
    pub state: String,
    pub last_latency_ms: Option<f64>,
    pub measurement_loss_ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TargetDetail {
    pub target_id: String,
    pub name: String,
    pub group_path: String,
    pub labels: serde_json::Value,
    pub state: String,
    pub checks: Vec<CheckSummary>,
}

pub async fn get_target(
    State(state): State<AppState>,
    Path(target_id): Path<String>,
) -> Result<Json<TargetDetail>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;

    let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, &target_id)
        .await
        .map_err(|e| ApiError {
            code: "internal_error".to_owned(),
            message: e.to_string(),
            request_id: make_request_id(),
        })?
        .ok_or_else(|| not_found("target_not_found", "The requested target does not exist"))?;

    let checks = kemuri_storage::CheckRepo::list_with_state(&pool, target.internal_id, observer_id)
        .await
        .map_err(|e| ApiError {
            code: "internal_error".to_owned(),
            message: e.to_string(),
            request_id: make_request_id(),
        })?;

    let check_summaries: Vec<CheckSummary> = checks
        .iter()
        .map(|c| CheckSummary {
            check_id: c.check_id.clone(),
            probe_type: c.probe_type.clone(),
            state: c.state.as_deref().unwrap_or("no_data").to_owned(),
            last_latency_ms: c.last_latency_ns.map(|ns| ns as f64 / 1_000_000.0),
            measurement_loss_ratio: c.last_measurement_loss_ratio,
        })
        .collect();

    let overall_state =
        check_summaries
            .iter()
            .map(|c| c.state.as_str())
            .fold("healthy", |acc, s| match (acc, s) {
                ("down", _) | (_, "down") => "down",
                ("degraded", _) | (_, "degraded") => "degraded",
                ("no_data", _) | (_, "no_data") => "no_data",
                _ => "healthy",
            });

    let labels: serde_json::Value = serde_json::from_str(&target.labels)
        .unwrap_or(serde_json::Value::Object(Default::default()));

    Ok(Json(TargetDetail {
        target_id: target.target_id,
        name: target.name,
        group_path: target.group_path,
        labels,
        state: overall_state.to_owned(),
        checks: check_summaries,
    }))
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckDetail {
    pub check_id: String,
    pub target_id: String,
    pub probe_type: String,
    pub state: String,
    pub last_latency_ms: Option<f64>,
    pub measurement_loss_ratio: Option<f64>,
    pub health_failure_ratio: Option<f64>,
    pub last_round_at: Option<String>,
    pub observer_id: String,
}

pub async fn list_checks(
    State(state): State<AppState>,
    Path(target_id): Path<String>,
) -> Result<Json<Vec<CheckSummary>>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;

    let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, &target_id)
        .await
        .map_err(|e| ApiError {
            code: "internal_error".to_owned(),
            message: e.to_string(),
            request_id: make_request_id(),
        })?
        .ok_or_else(|| not_found("target_not_found", "The requested target does not exist"))?;

    let checks = kemuri_storage::CheckRepo::list_with_state(&pool, target.internal_id, observer_id)
        .await
        .map_err(|e| ApiError {
            code: "internal_error".to_owned(),
            message: e.to_string(),
            request_id: make_request_id(),
        })?;

    let summaries: Vec<CheckSummary> = checks
        .iter()
        .map(|c| CheckSummary {
            check_id: c.check_id.clone(),
            probe_type: c.probe_type.clone(),
            state: c.state.as_deref().unwrap_or("no_data").to_owned(),
            last_latency_ms: c.last_latency_ns.map(|ns| ns as f64 / 1_000_000.0),
            measurement_loss_ratio: c.last_measurement_loss_ratio,
        })
        .collect();

    Ok(Json(summaries))
}

pub async fn get_check(
    State(state): State<AppState>,
    Path((target_id, check_id)): Path<(String, String)>,
) -> Result<Json<CheckDetail>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;

    let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, &target_id)
        .await
        .map_err(|e| ApiError {
            code: "internal_error".to_owned(),
            message: e.to_string(),
            request_id: make_request_id(),
        })?
        .ok_or_else(|| not_found("target_not_found", "The requested target does not exist"))?;

    let check = kemuri_storage::CheckRepo::get_with_state(
        &pool,
        target.internal_id,
        &check_id,
        observer_id,
    )
    .await
    .map_err(|e| ApiError {
        code: "internal_error".to_owned(),
        message: e.to_string(),
        request_id: make_request_id(),
    })?
    .ok_or_else(|| not_found("check_not_found", "The requested check does not exist"))?;

    Ok(Json(CheckDetail {
        check_id: check.check_id,
        target_id,
        probe_type: check.probe_type,
        state: check.state.as_deref().unwrap_or("no_data").to_owned(),
        last_latency_ms: check.last_latency_ns.map(|ns| ns as f64 / 1_000_000.0),
        measurement_loss_ratio: check.last_measurement_loss_ratio,
        health_failure_ratio: check.last_health_failure_ratio,
        last_round_at: check.last_round_at,
        observer_id: "local".to_owned(),
    }))
}

#[derive(Debug, Clone, Deserialize)]
pub struct SeriesQuery {
    pub from: String,
    pub to: String,
    pub max_points: Option<u32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeriesPoint {
    pub timestamp: String,
    pub rounds_count: usize,
    pub attempted: i64,
    pub latency_bearing: i64,
    pub healthy: i64,
    pub unhealthy: i64,
    pub measurement_lost: i64,
    pub min_latency_ms: Option<f64>,
    pub p50_latency_ms: Option<f64>,
    pub p95_latency_ms: Option<f64>,
    pub max_latency_ms: Option<f64>,
    pub measurement_loss_ratio: f64,
    pub health_failure_ratio: f64,
    pub histogram_bins: Vec<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SeriesResponse {
    pub target_id: String,
    pub check_id: String,
    pub observer_id: String,
    pub from: String,
    pub to: String,
    pub resolution_ms: i64,
    pub source: String,
    pub quantiles: String,
    pub histogram_bin_representatives_ms: Vec<f64>,
    pub points: Vec<SeriesPoint>,
}

pub async fn get_series(
    State(state): State<AppState>,
    Path((target_id, check_id)): Path<(String, String)>,
    Query(query): Query<SeriesQuery>,
) -> Result<Json<SeriesResponse>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;

    let max_points = query.max_points.unwrap_or(1000).min(5000);

    let from_time = chrono::DateTime::parse_from_rfc3339(&query.from)
        .map_err(|_| bad_request("invalid 'from' parameter, expected ISO 8601 format"))?;
    let to_time = chrono::DateTime::parse_from_rfc3339(&query.to)
        .map_err(|_| bad_request("invalid 'to' parameter, expected ISO 8601 format"))?;

    let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, &target_id)
        .await
        .map_err(|e| ApiError {
            code: "internal_error".to_owned(),
            message: e.to_string(),
            request_id: make_request_id(),
        })?
        .ok_or_else(|| not_found("target_not_found", "The requested target does not exist"))?;

    let check = kemuri_storage::CheckRepo::get_with_state(
        &pool,
        target.internal_id,
        &check_id,
        observer_id,
    )
    .await
    .map_err(|e| ApiError {
        code: "internal_error".to_owned(),
        message: e.to_string(),
        request_id: make_request_id(),
    })?
    .ok_or_else(|| not_found("check_not_found", "The requested check does not exist"))?;

    let range_secs = (to_time - from_time).num_seconds().max(1) as u64;

    let raw_count = kemuri_storage::RoundRepo::count_by_check_range(
        &pool,
        check.internal_id,
        observer_id,
        &query.from,
        &query.to,
    )
    .await
    .map_err(|e| ApiError {
        code: "internal_error".to_owned(),
        message: e.to_string(),
        request_id: make_request_id(),
    })? as u64;

    let threshold = (max_points as u64 * 3) / 2;

    let (resolution_secs, source, quantiles) = if raw_count <= threshold {
        (0i64, "raw", "exact")
    } else if range_secs / 300 <= threshold {
        (300i64, "rollup", "approximate")
    } else {
        (3600i64, "rollup", "approximate")
    };

    let points = if resolution_secs == 0 {
        build_series_from_raw(
            &pool,
            check.internal_id,
            observer_id,
            &from_time,
            &to_time,
            max_points,
        )
        .await?
    } else {
        build_series_from_rollups(
            &pool,
            check.internal_id,
            observer_id,
            resolution_secs,
            &from_time,
            &to_time,
            max_points,
        )
        .await?
    };

    let bin_reps_ns = kemuri_core::Histogram::bin_representatives();
    let bin_reps_ms: Vec<f64> = bin_reps_ns
        .iter()
        .map(|&ns| ns as f64 / 1_000_000.0)
        .collect();

    Ok(Json(SeriesResponse {
        target_id,
        check_id,
        observer_id: "local".to_owned(),
        from: query.from,
        to: query.to,
        resolution_ms: resolution_secs * 1000,
        source: source.to_owned(),
        quantiles: quantiles.to_owned(),
        histogram_bin_representatives_ms: bin_reps_ms,
        points,
    }))
}

async fn build_series_from_raw(
    pool: &sqlx::SqlitePool,
    check_internal_id: i64,
    observer_internal_id: i64,
    from_time: &chrono::DateTime<chrono::FixedOffset>,
    to_time: &chrono::DateTime<chrono::FixedOffset>,
    max_points: u32,
) -> Result<Vec<SeriesPoint>, ApiError> {
    let from_str = from_time.to_rfc3339();
    let to_str = to_time.to_rfc3339();

    let rounds = kemuri_storage::RoundRepo::query_by_check_range_with_observer(
        pool,
        check_internal_id,
        observer_internal_id,
        &from_str,
        &to_str,
    )
    .await
    .map_err(|e| ApiError {
        code: "internal_error".to_owned(),
        message: e.to_string(),
        request_id: make_request_id(),
    })?;

    let time_range = (*to_time - *from_time).num_seconds().max(1) as u64;
    let bucket_secs = (time_range / max_points as u64).max(1);

    let mut buckets: std::collections::BTreeMap<i64, Vec<&kemuri_storage::RoundRow>> =
        std::collections::BTreeMap::new();

    for round in &rounds {
        let scheduled = chrono::DateTime::parse_from_rfc3339(&round.scheduled_at);
        if let Ok(t) = scheduled {
            let bucket_idx = (t.timestamp() - from_time.timestamp()) / bucket_secs as i64;
            buckets.entry(bucket_idx).or_default().push(round);
        }
    }

    let points: Vec<SeriesPoint> = buckets
        .iter()
        .map(|(idx, bucket_rounds)| {
            let timestamp_secs = from_time.timestamp() + idx * bucket_secs as i64;
            let timestamp = chrono::DateTime::from_timestamp(timestamp_secs, 0)
                .unwrap_or_default()
                .to_rfc3339();

            let attempted: i64 = bucket_rounds
                .iter()
                .map(|r| r.attempted_samples as i64)
                .sum();
            let latency_bearing: i64 = bucket_rounds
                .iter()
                .map(|r| r.latency_bearing_samples as i64)
                .sum();
            let healthy: i64 = bucket_rounds.iter().map(|r| r.healthy_samples as i64).sum();
            let unhealthy: i64 = bucket_rounds
                .iter()
                .map(|r| r.unhealthy_samples as i64)
                .sum();
            let measurement_lost: i64 = bucket_rounds
                .iter()
                .map(|r| r.measurement_loss_samples as i64)
                .sum();

            let min_lat = bucket_rounds
                .iter()
                .filter_map(|r| r.min_latency_ns)
                .min()
                .map(|ns| ns as f64 / 1_000_000.0);

            let max_lat = bucket_rounds
                .iter()
                .filter_map(|r| r.max_latency_ns)
                .max()
                .map(|ns| ns as f64 / 1_000_000.0);

            let mut histogram = kemuri_core::Histogram::new();
            let mut all_lats: Vec<i64> = Vec::new();

            for round in bucket_rounds {
                if let Some(ref blob) = round.sample_blob
                    && let Ok(records) = kemuri_core::decode_samples(blob)
                {
                    for record in &records {
                        if let Some(lat_ns) = record.latency_ns {
                            histogram.record(lat_ns);
                            all_lats.push(lat_ns as i64);
                        }
                    }
                }
            }

            all_lats.sort();
            let p50 = percentile(&all_lats, 50).map(|ns| ns as f64 / 1_000_000.0);
            let p95 = percentile(&all_lats, 95).map(|ns| ns as f64 / 1_000_000.0);

            let total = (healthy + unhealthy + measurement_lost) as f64;
            let ml_ratio = if total > 0.0 {
                measurement_lost as f64 / total
            } else {
                0.0
            };
            let hf_ratio = if total > 0.0 {
                unhealthy as f64 / total
            } else {
                0.0
            };

            SeriesPoint {
                timestamp,
                rounds_count: bucket_rounds.len(),
                attempted,
                latency_bearing,
                healthy,
                unhealthy,
                measurement_lost,
                min_latency_ms: min_lat,
                p50_latency_ms: p50,
                p95_latency_ms: p95,
                max_latency_ms: max_lat,
                measurement_loss_ratio: ml_ratio,
                health_failure_ratio: hf_ratio,
                histogram_bins: histogram.bins().to_vec(),
            }
        })
        .collect();

    Ok(points)
}

async fn build_series_from_rollups(
    pool: &sqlx::SqlitePool,
    check_internal_id: i64,
    observer_internal_id: i64,
    resolution_secs: i64,
    from_time: &chrono::DateTime<chrono::FixedOffset>,
    to_time: &chrono::DateTime<chrono::FixedOffset>,
    max_points: u32,
) -> Result<Vec<SeriesPoint>, ApiError> {
    let from_str = from_time.to_rfc3339();
    let to_str = to_time.to_rfc3339();

    let rollups = kemuri_storage::RollupRepo::query_by_check_and_range(
        pool,
        check_internal_id,
        observer_internal_id,
        resolution_secs,
        &from_str,
        &to_str,
    )
    .await
    .map_err(|e| ApiError {
        code: "internal_error".to_owned(),
        message: e.to_string(),
        request_id: make_request_id(),
    })?;

    if rollups.len() <= max_points as usize {
        let points: Vec<SeriesPoint> = rollups.iter().map(rollup_to_series_point).collect();
        return Ok(points);
    }

    let merge_factor = (rollups.len() / max_points as usize).max(1);
    let mut points = Vec::new();
    let mut i = 0;
    while i < rollups.len() {
        let end = (i + merge_factor).min(rollups.len());
        let chunk = &rollups[i..end];
        points.push(merge_rollups(chunk));
        i = end;
    }

    Ok(points)
}

fn rollup_to_series_point(r: &kemuri_storage::RollupRow) -> SeriesPoint {
    let histogram = r
        .histogram_blob
        .as_ref()
        .and_then(|blob| kemuri_core::Histogram::decode(blob))
        .unwrap_or_default();

    let p50 = histogram.quantile(0.5).map(|ns| ns as f64 / 1_000_000.0);
    let p95 = histogram.quantile(0.95).map(|ns| ns as f64 / 1_000_000.0);

    let total = (r.healthy_samples + r.unhealthy_samples + r.measurement_loss_samples) as f64;
    let ml_ratio = if total > 0.0 {
        r.measurement_loss_samples as f64 / total
    } else {
        0.0
    };
    let hf_ratio = if total > 0.0 {
        r.unhealthy_samples as f64 / total
    } else {
        0.0
    };

    SeriesPoint {
        timestamp: r.bucket_start.clone(),
        rounds_count: r.scheduled_rounds as usize,
        attempted: r.attempted_samples,
        latency_bearing: r.latency_bearing_samples,
        healthy: r.healthy_samples,
        unhealthy: r.unhealthy_samples,
        measurement_lost: r.measurement_loss_samples,
        min_latency_ms: r.min_latency_ns.map(|ns| ns as f64 / 1_000_000.0),
        p50_latency_ms: p50,
        p95_latency_ms: p95,
        max_latency_ms: r.max_latency_ns.map(|ns| ns as f64 / 1_000_000.0),
        measurement_loss_ratio: ml_ratio,
        health_failure_ratio: hf_ratio,
        histogram_bins: histogram.bins().to_vec(),
    }
}

fn merge_rollups(chunk: &[kemuri_storage::RollupRow]) -> SeriesPoint {
    if chunk.len() == 1 {
        return rollup_to_series_point(&chunk[0]);
    }

    let first = &chunk[0];
    let mut histogram = kemuri_core::Histogram::new();
    let mut total_attempted: i64 = 0;
    let mut total_latency_bearing: i64 = 0;
    let mut total_healthy: i64 = 0;
    let mut total_unhealthy: i64 = 0;
    let mut total_measurement_lost: i64 = 0;
    let mut total_rounds: usize = 0;
    let mut min_lat: Option<i64> = None;
    let mut max_lat: Option<i64> = None;

    for r in chunk {
        total_rounds += r.scheduled_rounds as usize;
        total_attempted += r.attempted_samples;
        total_latency_bearing += r.latency_bearing_samples;
        total_healthy += r.healthy_samples;
        total_unhealthy += r.unhealthy_samples;
        total_measurement_lost += r.measurement_loss_samples;

        if let Some(min) = r.min_latency_ns {
            min_lat = Some(min_lat.map_or(min, |m: i64| m.min(min)));
        }
        if let Some(max) = r.max_latency_ns {
            max_lat = Some(max_lat.map_or(max, |m: i64| m.max(max)));
        }

        if let Some(ref blob) = r.histogram_blob
            && let Some(other) = kemuri_core::Histogram::decode(blob)
        {
            histogram.merge(&other);
        }
    }

    let p50 = histogram.quantile(0.5).map(|ns| ns as f64 / 1_000_000.0);
    let p95 = histogram.quantile(0.95).map(|ns| ns as f64 / 1_000_000.0);

    let total = (total_healthy + total_unhealthy + total_measurement_lost) as f64;
    let ml_ratio = if total > 0.0 {
        total_measurement_lost as f64 / total
    } else {
        0.0
    };
    let hf_ratio = if total > 0.0 {
        total_unhealthy as f64 / total
    } else {
        0.0
    };

    SeriesPoint {
        timestamp: first.bucket_start.clone(),
        rounds_count: total_rounds,
        attempted: total_attempted,
        latency_bearing: total_latency_bearing,
        healthy: total_healthy,
        unhealthy: total_unhealthy,
        measurement_lost: total_measurement_lost,
        min_latency_ms: min_lat.map(|ns| ns as f64 / 1_000_000.0),
        p50_latency_ms: p50,
        p95_latency_ms: p95,
        max_latency_ms: max_lat.map(|ns| ns as f64 / 1_000_000.0),
        measurement_loss_ratio: ml_ratio,
        health_failure_ratio: hf_ratio,
        histogram_bins: histogram.bins().to_vec(),
    }
}

fn percentile(sorted: &[i64], p: u32) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    let idx = ((p as f64 / 100.0) * (sorted.len() - 1) as f64).round() as usize;
    Some(sorted[idx.min(sorted.len() - 1)])
}

#[derive(Debug, Clone, Deserialize)]
pub struct RoundsQuery {
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SampleDetail {
    pub outcome: String,
    pub latency_ms: Option<f64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoundSummary {
    pub scheduled_at: String,
    pub execution_status: String,
    pub stop_reason: Option<String>,
    pub attempted_samples: i32,
    pub healthy_samples: i32,
    pub unhealthy_samples: i32,
    pub measurement_loss_samples: i32,
    pub min_latency_ms: Option<f64>,
    pub max_latency_ms: Option<f64>,
    pub outcome_summary: Option<String>,
    pub samples: Vec<SampleDetail>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RoundsResponse {
    pub rounds: Vec<RoundSummary>,
    pub next_cursor: Option<String>,
}

pub async fn get_rounds(
    State(state): State<AppState>,
    Path((target_id, check_id)): Path<(String, String)>,
    Query(query): Query<RoundsQuery>,
) -> Result<Json<RoundsResponse>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;
    let limit = query.limit.unwrap_or(50).min(200);

    let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, &target_id)
        .await
        .map_err(|e| ApiError {
            code: "internal_error".to_owned(),
            message: e.to_string(),
            request_id: make_request_id(),
        })?
        .ok_or_else(|| not_found("target_not_found", "The requested target does not exist"))?;

    let check = kemuri_storage::CheckRepo::get_with_state(
        &pool,
        target.internal_id,
        &check_id,
        observer_id,
    )
    .await
    .map_err(|e| ApiError {
        code: "internal_error".to_owned(),
        message: e.to_string(),
        request_id: make_request_id(),
    })?
    .ok_or_else(|| not_found("check_not_found", "The requested check does not exist"))?;

    let rounds = kemuri_storage::RoundRepo::query_recent_by_check(
        &pool,
        check.internal_id,
        observer_id,
        limit + 1,
        query.cursor.as_deref(),
    )
    .await
    .map_err(|e| ApiError {
        code: "internal_error".to_owned(),
        message: e.to_string(),
        request_id: make_request_id(),
    })?;

    let has_more = rounds.len() > limit as usize;
    let rounds = if has_more {
        &rounds[..limit as usize]
    } else {
        &rounds[..]
    };

    let summaries: Vec<RoundSummary> = rounds
        .iter()
        .map(|r| {
            let samples = decode_sample_details(r.sample_blob.as_deref());
            RoundSummary {
                scheduled_at: r.scheduled_at.clone(),
                execution_status: r.execution_status.clone(),
                stop_reason: r.stop_reason.clone(),
                attempted_samples: r.attempted_samples,
                healthy_samples: r.healthy_samples,
                unhealthy_samples: r.unhealthy_samples,
                measurement_loss_samples: r.measurement_loss_samples,
                min_latency_ms: r.min_latency_ns.map(|ns| ns as f64 / 1_000_000.0),
                max_latency_ms: r.max_latency_ns.map(|ns| ns as f64 / 1_000_000.0),
                outcome_summary: r.outcome_summary.clone(),
                samples,
            }
        })
        .collect();

    let next_cursor = if has_more {
        rounds.last().map(|r| r.scheduled_at.clone())
    } else {
        None
    };

    Ok(Json(RoundsResponse {
        rounds: summaries,
        next_cursor,
    }))
}

fn decode_sample_details(sample_blob: Option<&[u8]>) -> Vec<SampleDetail> {
    let blob = match sample_blob {
        Some(b) => b,
        None => return Vec::new(),
    };

    let records = match kemuri_core::decode_samples(blob) {
        Ok(r) => r,
        Err(_) => return Vec::new(),
    };

    records
        .into_iter()
        .map(|rec| {
            let metadata = rec.metadata.and_then(|bytes| {
                serde_json::from_slice::<std::collections::HashMap<String, String>>(&bytes)
                    .ok()
                    .map(|map| {
                        let mut obj = serde_json::Map::new();
                        for (k, v) in map {
                            obj.insert(k, serde_json::Value::String(v));
                        }
                        serde_json::Value::Object(obj)
                    })
                    .or_else(|| {
                        String::from_utf8(bytes.clone())
                            .ok()
                            .map(|s| serde_json::json!({ "detail": s }))
                    })
            });

            SampleDetail {
                outcome: format!("{:?}", rec.outcome),
                latency_ms: rec.latency_ns.map(|ns| ns as f64 / 1_000_000.0),
                metadata,
            }
        })
        .collect()
}

pub async fn list_groups(
    State(state): State<AppState>,
) -> Result<Json<Vec<GroupResponse>>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;

    let rows = kemuri_storage::TargetRepo::list_with_state(&pool, observer_id)
        .await
        .map_err(|e| ApiError {
            code: "internal_error".to_owned(),
            message: e.to_string(),
            request_id: make_request_id(),
        })?;

    let mut group_map: std::collections::HashMap<String, Vec<TargetSummary>> =
        std::collections::HashMap::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for row in &rows {
        if seen.insert(row.target_id.clone()) {
            let group = if row.group_path.is_empty() {
                "default".to_owned()
            } else {
                row.group_path.clone()
            };
            let state_str = row.state.as_deref().unwrap_or("no_data");
            let summary = TargetSummary {
                target_id: row.target_id.clone(),
                name: row.name.clone(),
                group_path: group.clone(),
                state: state_str.to_owned(),
                checks_count: 0,
            };
            group_map.entry(group).or_default().push(summary);
        }
    }

    let groups: Vec<GroupResponse> = group_map
        .into_iter()
        .map(|(group_path, targets)| GroupResponse {
            group_path,
            targets,
        })
        .collect();

    Ok(Json(groups))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollup_to_series_point_basic() {
        let row = kemuri_storage::RollupRow {
            check_internal_id: 1,
            observer_internal_id: 1,
            resolution_seconds: 300,
            bucket_start: "2024-01-01T00:00:00Z".to_owned(),
            scheduled_rounds: 10,
            completed_rounds: 9,
            partial_rounds: 1,
            configured_sample_slots: 30,
            attempted_samples: 30,
            latency_bearing_samples: 28,
            healthy_samples: 25,
            unhealthy_samples: 3,
            measurement_loss_samples: 2,
            outcome_counts: "{}".to_owned(),
            min_latency_ns: Some(1_000_000),
            max_latency_ns: Some(50_000_000),
            sum_latency_ns: 200_000_000,
            histogram_version: 1,
            histogram_blob: None,
            no_data_counts: "{}".to_owned(),
        };

        let point = rollup_to_series_point(&row);
        assert_eq!(point.rounds_count, 10);
        assert_eq!(point.healthy, 25);
        assert_eq!(point.unhealthy, 3);
        assert_eq!(point.measurement_lost, 2);
        assert!((point.measurement_loss_ratio - 0.0667).abs() < 0.01);
        assert!((point.health_failure_ratio - 0.1).abs() < 0.01);
        assert_eq!(
            point.histogram_bins.len(),
            kemuri_core::Histogram::num_bins()
        );
    }

    #[test]
    fn merge_rollups_combines_counters() {
        let mut h1 = kemuri_core::Histogram::new();
        h1.record(1_000_000);
        h1.record(2_000_000);

        let mut h2 = kemuri_core::Histogram::new();
        h2.record(5_000_000);

        let row1 = kemuri_storage::RollupRow {
            check_internal_id: 1,
            observer_internal_id: 1,
            resolution_seconds: 300,
            bucket_start: "2024-01-01T00:00:00Z".to_owned(),
            scheduled_rounds: 5,
            completed_rounds: 5,
            partial_rounds: 0,
            configured_sample_slots: 15,
            attempted_samples: 15,
            latency_bearing_samples: 14,
            healthy_samples: 14,
            unhealthy_samples: 0,
            measurement_loss_samples: 1,
            outcome_counts: "{}".to_owned(),
            min_latency_ns: Some(1_000_000),
            max_latency_ns: Some(10_000_000),
            sum_latency_ns: 50_000_000,
            histogram_version: 1,
            histogram_blob: Some(h1.encode()),
            no_data_counts: "{}".to_owned(),
        };

        let row2 = kemuri_storage::RollupRow {
            check_internal_id: 1,
            observer_internal_id: 1,
            resolution_seconds: 300,
            bucket_start: "2024-01-01T00:05:00Z".to_owned(),
            scheduled_rounds: 5,
            completed_rounds: 4,
            partial_rounds: 1,
            configured_sample_slots: 15,
            attempted_samples: 15,
            latency_bearing_samples: 10,
            healthy_samples: 8,
            unhealthy_samples: 2,
            measurement_loss_samples: 5,
            outcome_counts: "{}".to_owned(),
            min_latency_ns: Some(3_000_000),
            max_latency_ns: Some(20_000_000),
            sum_latency_ns: 80_000_000,
            histogram_version: 1,
            histogram_blob: Some(h2.encode()),
            no_data_counts: "{}".to_owned(),
        };

        let merged = merge_rollups(&[row1, row2]);
        assert_eq!(merged.rounds_count, 10);
        assert_eq!(merged.healthy, 22);
        assert_eq!(merged.unhealthy, 2);
        assert_eq!(merged.measurement_lost, 6);
        assert_eq!(merged.min_latency_ms, Some(1.0));
        assert_eq!(merged.max_latency_ms, Some(20.0));
    }

    #[test]
    fn series_response_has_metadata() {
        let bin_reps_ns = kemuri_core::Histogram::bin_representatives();
        let bin_reps_ms: Vec<f64> = bin_reps_ns
            .iter()
            .map(|&ns| ns as f64 / 1_000_000.0)
            .collect();
        assert_eq!(bin_reps_ms.len(), kemuri_core::Histogram::num_bins());
        assert!(bin_reps_ms[0] > 0.0);
    }
}
