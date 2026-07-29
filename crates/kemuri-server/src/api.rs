use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::http::header::{HeaderName, HeaderValue};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::AppState;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ApiError {
    pub code: String,
    pub message: String,
    pub request_id: String,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.code.as_str() {
            "target_not_found" | "check_not_found" | "alert_not_found" | "group_not_found"
            | "route_not_found" => StatusCode::NOT_FOUND,
            "bad_request" | "invalid_params" => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let request_id = self.request_id.clone();
        let mut response = (status, Json(self)).into_response();
        if let Ok(value) = HeaderValue::from_str(&request_id) {
            response
                .headers_mut()
                .insert(HeaderName::from_static("x-request-id"), value);
        }
        response
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

pub(crate) fn not_found_public(code: &str, message: &str) -> ApiError {
    not_found(code, message)
}

fn bad_request(message: &str) -> ApiError {
    ApiError {
        code: "bad_request".to_owned(),
        message: message.to_owned(),
        request_id: make_request_id(),
    }
}

pub(crate) fn bad_request_public(message: &str) -> ApiError {
    bad_request(message)
}

fn internal_error(e: impl std::fmt::Display) -> ApiError {
    tracing::error!(error = %e, "API internal error");
    ApiError {
        code: "internal_error".to_owned(),
        message: "An internal error occurred".to_owned(),
        request_id: make_request_id(),
    }
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AlertsQuery {
    pub state: Option<String>,
    pub rule_id: Option<String>,
    pub target_id: Option<String>,
    pub check_id: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AlertStateResponse {
    pub internal_id: i64,
    pub rule_id: String,
    pub target_id: String,
    pub check_id: String,
    pub state: String,
    pub state_entered_ms: i64,
    pub first_condition_true_ms: Option<i64>,
    pub last_evaluated_ms: Option<i64>,
    pub last_notification_ms: Option<i64>,
    pub last_metric_value: Option<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AlertsListResponse {
    pub alerts: Vec<AlertStateResponse>,
    pub next_cursor: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/alerts",
    params(AlertsQuery),
    responses(
        (status = 200, body = AlertsListResponse),
        (status = 400, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
pub async fn list_alerts(
    State(state): State<AppState>,
    Query(query): Query<AlertsQuery>,
) -> Result<Json<AlertsListResponse>, ApiError> {
    let pool = state.pool.clone();
    let limit = validate_limit(query.limit, 100)?;
    let cursor = decode_numeric_cursor(query.cursor.as_deref())?;

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
            state_entered_ms: timestamp_millis(&alert.state_entered_at),
            first_condition_true_ms: alert
                .first_condition_true_at
                .as_deref()
                .map(timestamp_millis),
            last_evaluated_ms: alert.last_evaluated_at.as_deref().map(timestamp_millis),
            last_notification_ms: alert.last_notification_at.as_deref().map(timestamp_millis),
            last_metric_value: alert.last_metric_value,
        });
    }

    responses.sort_by_key(|alert| std::cmp::Reverse(alert.internal_id));
    if let Some(cursor) = cursor {
        responses.retain(|alert| alert.internal_id < cursor);
    }
    let next_cursor = page_by_numeric_id(&mut responses, limit, |alert| alert.internal_id);

    Ok(Json(AlertsListResponse {
        alerts: responses,
        next_cursor,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/alerts/{alert_id}",
    params(("alert_id" = i64, Path, description = "Alert state identifier")),
    responses(
        (status = 200, body = AlertStateResponse),
        (status = 404, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
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
        state_entered_ms: timestamp_millis(&alert.state_entered_at),
        first_condition_true_ms: alert
            .first_condition_true_at
            .as_deref()
            .map(timestamp_millis),
        last_evaluated_ms: alert.last_evaluated_at.as_deref().map(timestamp_millis),
        last_notification_ms: alert.last_notification_at.as_deref().map(timestamp_millis),
        last_metric_value: alert.last_metric_value,
    }))
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AlertEventsQuery {
    pub rule_id: Option<String>,
    pub target_id: Option<String>,
    pub check_id: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
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
    pub timestamp_ms: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AlertEventsResponse {
    pub events: Vec<AlertEventResponse>,
    pub next_cursor: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/alert-events",
    params(AlertEventsQuery),
    responses(
        (status = 200, body = AlertEventsResponse),
        (status = 400, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
pub async fn list_alert_events(
    State(state): State<AppState>,
    Query(query): Query<AlertEventsQuery>,
) -> Result<Json<AlertEventsResponse>, ApiError> {
    let pool = state.pool.clone();
    let limit = validate_limit(query.limit, 100)?;
    let cursor = decode_numeric_cursor(query.cursor.as_deref())?;

    let events = if let Some(ref rule_id) = query.rule_id {
        kemuri_storage::AlertEventRepo::list_by_rule(&pool, rule_id, limit)
            .await
            .map_err(internal_error)?
    } else if let Some(from_ms) = query.from_ms {
        let from = parse_range_time(Some(from_ms), "from_ms")?.to_rfc3339();
        let to =
            parse_range_time(Some(query.to_ms.unwrap_or(4_102_444_800_000)), "to_ms")?.to_rfc3339();
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
            &from,
            &to,
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
            timestamp_ms: timestamp_millis(&event.occurred_at),
        });
    }

    responses.sort_by_key(|event| std::cmp::Reverse(event.internal_id));
    if let Some(cursor) = cursor {
        responses.retain(|event| event.internal_id < cursor);
    }
    let next_cursor = page_by_numeric_id(&mut responses, limit, |event| event.internal_id);

    Ok(Json(AlertEventsResponse {
        events: responses,
        next_cursor,
    }))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GroupResponse {
    pub group_path: String,
    pub targets: Vec<TargetSummary>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TargetSummary {
    pub target_id: String,
    pub name: String,
    pub group_path: String,
    pub state: String,
    pub checks_count: usize,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TargetsResponse {
    pub targets: Vec<TargetSummary>,
    pub groups: Vec<GroupResponse>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PageQuery {
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/targets",
    params(PageQuery),
    responses(
        (status = 200, body = TargetsResponse),
        (status = 400, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
pub async fn list_targets(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<TargetsResponse>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;
    let limit = validate_limit(query.limit, 100)?;
    let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;

    let rows = kemuri_storage::TargetRepo::list_with_state(&pool, observer_id)
        .await
        .map_err(internal_error)?;

    let mut target_map: std::collections::HashMap<String, Vec<TargetSummary>> =
        std::collections::HashMap::new();
    let mut all_targets: Vec<TargetSummary> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut counts = std::collections::HashMap::new();
    let mut worst_states: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for row in &rows {
        *counts.entry(row.target_id.clone()).or_insert(0usize) += 1;
        let candidate = row.state.as_deref().unwrap_or("no_data");
        let current = worst_states
            .entry(row.target_id.clone())
            .or_insert_with(|| candidate.to_owned());
        if state_rank(candidate) > state_rank(current) {
            *current = candidate.to_owned();
        }
    }

    for row in &rows {
        if seen.insert(row.target_id.clone()) {
            let group = if row.group_path.is_empty() {
                "default".to_owned()
            } else {
                row.group_path.clone()
            };
            let state_str = worst_states
                .get(&row.target_id)
                .map(String::as_str)
                .unwrap_or("no_data");
            let summary = TargetSummary {
                target_id: row.target_id.clone(),
                name: row.name.clone(),
                group_path: group.clone(),
                state: state_str.to_owned(),
                checks_count: counts.get(&row.target_id).copied().unwrap_or(0),
            };
            target_map.entry(group).or_default().push(summary.clone());
            all_targets.push(summary);
        }
    }

    all_targets.sort_by(|left, right| left.target_id.cmp(&right.target_id));
    if let Some(cursor) = cursor {
        all_targets.retain(|target| target.target_id > cursor);
    }
    let has_more = all_targets.len() > limit as usize;
    all_targets.truncate(limit as usize);
    let selected: std::collections::HashSet<&str> = all_targets
        .iter()
        .map(|target| target.target_id.as_str())
        .collect();
    let next_cursor = has_more.then(|| {
        hex::encode(
            all_targets
                .last()
                .map(|target| target.target_id.as_bytes())
                .unwrap_or_default(),
        )
    });

    let mut groups: Vec<GroupResponse> = target_map
        .into_iter()
        .filter_map(|(group_path, mut targets)| {
            targets.retain(|target| selected.contains(target.target_id.as_str()));
            (!targets.is_empty()).then_some(GroupResponse {
                group_path,
                targets,
            })
        })
        .collect();
    groups.sort_by(|left, right| left.group_path.cmp(&right.group_path));

    Ok(Json(TargetsResponse {
        targets: all_targets,
        groups,
        next_cursor,
    }))
}

fn state_rank(state: &str) -> u8 {
    match state {
        "down" => 4,
        "degraded" => 3,
        "no_data" | "unknown" => 2,
        "healthy" => 1,
        _ => 0,
    }
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CheckSummary {
    pub check_id: String,
    pub probe_type: String,
    pub state: String,
    pub last_latency_us: Option<i64>,
    pub measurement_loss_ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct TargetDetail {
    pub target_id: String,
    pub name: String,
    pub group_path: String,
    pub labels: serde_json::Value,
    pub state: String,
    pub checks: Vec<CheckSummary>,
}

#[utoipa::path(
    get,
    path = "/api/v1/targets/{target_id}",
    params(("target_id" = String, Path, description = "Target identifier")),
    responses(
        (status = 200, body = TargetDetail),
        (status = 404, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
pub async fn get_target(
    State(state): State<AppState>,
    Path(target_id): Path<String>,
) -> Result<Json<TargetDetail>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;

    let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, &target_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| not_found("target_not_found", "The requested target does not exist"))?;

    let checks = kemuri_storage::CheckRepo::list_with_state(&pool, target.internal_id, observer_id)
        .await
        .map_err(internal_error)?;

    let check_summaries: Vec<CheckSummary> = checks
        .iter()
        .map(|c| CheckSummary {
            check_id: c.check_id.clone(),
            probe_type: c.probe_type.clone(),
            state: c.state.as_deref().unwrap_or("no_data").to_owned(),
            last_latency_us: c.last_latency_ns.map(|ns| ns / 1_000),
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

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CheckDetail {
    pub check_id: String,
    pub target_id: String,
    pub probe_type: String,
    pub state: String,
    pub last_latency_us: Option<i64>,
    pub measurement_loss_ratio: Option<f64>,
    pub health_failure_ratio: Option<f64>,
    pub last_round_timestamp_ms: Option<i64>,
    pub observer_id: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/targets/{target_id}/checks",
    params(
        ("target_id" = String, Path, description = "Target identifier"),
        PageQuery
    ),
    responses(
        (status = 200, body = ChecksResponse),
        (status = 400, body = ApiError),
        (status = 404, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
pub async fn list_checks(
    State(state): State<AppState>,
    Path(target_id): Path<String>,
    Query(query): Query<PageQuery>,
) -> Result<Json<ChecksResponse>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;
    let limit = validate_limit(query.limit, 100)?;
    let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;

    let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, &target_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| not_found("target_not_found", "The requested target does not exist"))?;

    let checks = kemuri_storage::CheckRepo::list_with_state(&pool, target.internal_id, observer_id)
        .await
        .map_err(internal_error)?;

    let mut summaries: Vec<CheckSummary> = checks
        .iter()
        .map(|c| CheckSummary {
            check_id: c.check_id.clone(),
            probe_type: c.probe_type.clone(),
            state: c.state.as_deref().unwrap_or("no_data").to_owned(),
            last_latency_us: c.last_latency_ns.map(|ns| ns / 1_000),
            measurement_loss_ratio: c.last_measurement_loss_ratio,
        })
        .collect();

    summaries.sort_by(|left, right| left.check_id.cmp(&right.check_id));
    if let Some(cursor) = cursor {
        summaries.retain(|check| check.check_id > cursor);
    }
    let has_more = summaries.len() > limit as usize;
    summaries.truncate(limit as usize);
    let next_cursor = has_more.then(|| {
        hex::encode(
            summaries
                .last()
                .map(|check| check.check_id.as_bytes())
                .unwrap_or_default(),
        )
    });

    Ok(Json(ChecksResponse {
        checks: summaries,
        next_cursor,
    }))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ChecksResponse {
    pub checks: Vec<CheckSummary>,
    pub next_cursor: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/targets/{target_id}/checks/{check_id}",
    params(
        ("target_id" = String, Path, description = "Target identifier"),
        ("check_id" = String, Path, description = "Check identifier")
    ),
    responses(
        (status = 200, body = CheckDetail),
        (status = 404, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
pub async fn get_check(
    State(state): State<AppState>,
    Path((target_id, check_id)): Path<(String, String)>,
) -> Result<Json<CheckDetail>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;

    let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, &target_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| not_found("target_not_found", "The requested target does not exist"))?;

    let check = kemuri_storage::CheckRepo::get_with_state(
        &pool,
        target.internal_id,
        &check_id,
        observer_id,
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(|| not_found("check_not_found", "The requested check does not exist"))?;

    Ok(Json(CheckDetail {
        check_id: check.check_id,
        target_id,
        probe_type: check.probe_type,
        state: check.state.as_deref().unwrap_or("no_data").to_owned(),
        last_latency_us: check.last_latency_ns.map(|ns| ns / 1_000),
        measurement_loss_ratio: check.last_measurement_loss_ratio,
        health_failure_ratio: check.last_health_failure_ratio,
        last_round_timestamp_ms: check.last_round_at.as_deref().map(timestamp_millis),
        observer_id: "local".to_owned(),
    }))
}

#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SeriesQuery {
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub max_points: Option<u32>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SeriesPoint {
    pub timestamp_ms: i64,
    pub bucket_status: String,
    pub rounds_count: usize,
    pub attempted: i64,
    pub latency_bearing: i64,
    pub healthy: i64,
    pub unhealthy: i64,
    pub measurement_lost: i64,
    pub min_latency_us: Option<i64>,
    pub p50_latency_us: Option<i64>,
    pub p95_latency_us: Option<i64>,
    pub max_latency_us: Option<i64>,
    pub measurement_loss_ratio: f64,
    pub health_failure_ratio: f64,
    pub histogram_bins: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SeriesResponse {
    pub target_id: String,
    pub check_id: String,
    pub observer_id: String,
    pub from_ms: i64,
    pub to_ms: i64,
    pub resolution_ms: i64,
    pub source: String,
    pub quantiles: String,
    pub histogram_bin_representatives_us: Vec<i64>,
    pub points: Vec<SeriesPoint>,
    pub alert_events: Vec<SeriesAlertEvent>,
    pub revision_markers: Vec<SeriesRevisionMarker>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SeriesAlertEvent {
    pub timestamp_ms: i64,
    pub event_type: String,
    pub rule_id: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SeriesRevisionMarker {
    pub timestamp_ms: i64,
    pub revision_id: String,
}

#[utoipa::path(
    get,
    path = "/api/v1/targets/{target_id}/checks/{check_id}/series",
    params(
        ("target_id" = String, Path, description = "Target identifier"),
        ("check_id" = String, Path, description = "Check identifier"),
        SeriesQuery
    ),
    responses(
        (status = 200, body = SeriesResponse),
        (status = 400, body = ApiError),
        (status = 404, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
pub async fn get_series(
    State(state): State<AppState>,
    Path((target_id, check_id)): Path<(String, String)>,
    Query(query): Query<SeriesQuery>,
) -> Result<Json<SeriesResponse>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;

    let max_points = query.max_points.unwrap_or(1000);
    if !(1..=5000).contains(&max_points) {
        return Err(bad_request("max_points must be between 1 and 5000"));
    }

    let from_time = parse_range_time(query.from_ms, "from_ms")?;
    let to_time = parse_range_time(query.to_ms, "to_ms")?;
    if from_time >= to_time {
        return Err(bad_request("from_ms must be less than to_ms"));
    }
    let from_string = from_time.to_rfc3339();
    let to_string = to_time.to_rfc3339();

    let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, &target_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| not_found("target_not_found", "The requested target does not exist"))?;

    let check = kemuri_storage::CheckRepo::get_with_state(
        &pool,
        target.internal_id,
        &check_id,
        observer_id,
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(|| not_found("check_not_found", "The requested check does not exist"))?;

    let range_secs = (to_time - from_time).num_seconds().max(1) as u64;

    let raw_count = kemuri_storage::RoundRepo::count_by_check_range(
        &pool,
        check.internal_id,
        observer_id,
        &from_string,
        &to_string,
    )
    .await
    .map_err(internal_error)? as u64;

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
    let bin_reps_us: Vec<i64> = bin_reps_ns.iter().map(|&ns| (ns / 1_000) as i64).collect();
    let alert_rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT occurred_at, event_type, rule_id FROM alert_events
         WHERE check_internal_id = ? AND occurred_at >= ? AND occurred_at < ?
         ORDER BY occurred_at",
    )
    .bind(check.internal_id)
    .bind(&from_string)
    .bind(&to_string)
    .fetch_all(&pool)
    .await
    .map_err(internal_error)?;
    let revision_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT strftime('%Y-%m-%dT%H:%M:%SZ', effective_at), revision_id FROM check_revisions
         WHERE check_internal_id = ? AND effective_at >= ? AND effective_at < ?
         ORDER BY effective_at",
    )
    .bind(check.internal_id)
    .bind(&from_string)
    .bind(&to_string)
    .fetch_all(&pool)
    .await
    .map_err(internal_error)?;

    Ok(Json(SeriesResponse {
        target_id,
        check_id,
        observer_id: "local".to_owned(),
        from_ms: from_time.timestamp_millis(),
        to_ms: to_time.timestamp_millis(),
        resolution_ms: resolution_secs * 1000,
        source: source.to_owned(),
        quantiles: quantiles.to_owned(),
        histogram_bin_representatives_us: bin_reps_us,
        points,
        alert_events: alert_rows
            .into_iter()
            .map(|(timestamp, event_type, rule_id)| SeriesAlertEvent {
                timestamp_ms: timestamp_millis(&timestamp),
                event_type,
                rule_id,
            })
            .collect(),
        revision_markers: revision_rows
            .into_iter()
            .map(|(timestamp, revision_id)| SeriesRevisionMarker {
                timestamp_ms: timestamp_millis(&timestamp),
                revision_id,
            })
            .collect(),
    }))
}

fn parse_range_time(
    milliseconds: Option<i64>,
    name: &str,
) -> Result<chrono::DateTime<chrono::FixedOffset>, ApiError> {
    chrono::DateTime::from_timestamp_millis(
        milliseconds.ok_or_else(|| bad_request(&format!("missing '{name}' parameter")))?,
    )
    .map(|value| value.fixed_offset())
    .ok_or_else(|| bad_request(&format!("invalid '{name}' parameter")))
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
    .map_err(internal_error)?;

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

    let observed_points: Vec<SeriesPoint> = buckets
        .iter()
        .map(|(idx, bucket_rounds)| {
            let timestamp_secs = from_time.timestamp() + idx * bucket_secs as i64;
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
                .map(|ns| ns / 1_000);

            let max_lat = bucket_rounds
                .iter()
                .filter_map(|r| r.max_latency_ns)
                .max()
                .map(|ns| ns / 1_000);

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
            let p50 = percentile(&all_lats, 50).map(|ns| ns / 1_000);
            let p95 = percentile(&all_lats, 95).map(|ns| ns / 1_000);

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
                timestamp_ms: timestamp_secs * 1_000,
                bucket_status: if bucket_rounds.iter().all(|round| {
                    round.execution_status.starts_with("skipped")
                        || round.execution_status == "cancelled"
                }) {
                    "skipped".to_owned()
                } else {
                    "observed".to_owned()
                },
                rounds_count: bucket_rounds.len(),
                attempted,
                latency_bearing,
                healthy,
                unhealthy,
                measurement_lost,
                min_latency_us: min_lat,
                p50_latency_us: p50,
                p95_latency_us: p95,
                max_latency_us: max_lat,
                measurement_loss_ratio: ml_ratio,
                health_failure_ratio: hf_ratio,
                histogram_bins: histogram.bins().to_vec(),
            }
        })
        .collect();

    let mut by_timestamp: std::collections::HashMap<i64, SeriesPoint> = observed_points
        .into_iter()
        .map(|point| (point.timestamp_ms, point))
        .collect();
    let bucket_count = time_range.div_ceil(bucket_secs).min(max_points as u64);
    let mut points = Vec::with_capacity(bucket_count as usize);
    for index in 0..bucket_count {
        let timestamp_ms = from_time.timestamp() * 1000 + (index * bucket_secs * 1000) as i64;
        points.push(
            by_timestamp
                .remove(&timestamp_ms)
                .unwrap_or_else(|| empty_series_point(timestamp_ms)),
        );
    }
    Ok(points)
}

fn empty_series_point(timestamp_ms: i64) -> SeriesPoint {
    SeriesPoint {
        timestamp_ms,
        bucket_status: "missing".to_owned(),
        rounds_count: 0,
        attempted: 0,
        latency_bearing: 0,
        healthy: 0,
        unhealthy: 0,
        measurement_lost: 0,
        min_latency_us: None,
        p50_latency_us: None,
        p95_latency_us: None,
        max_latency_us: None,
        measurement_loss_ratio: 0.0,
        health_failure_ratio: 0.0,
        histogram_bins: vec![0; kemuri_core::Histogram::bin_representatives().len()],
    }
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
    .map_err(internal_error)?;

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

    let p50 = histogram.quantile(0.5).map(|ns| (ns / 1_000) as i64);
    let p95 = histogram.quantile(0.95).map(|ns| (ns / 1_000) as i64);

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
        timestamp_ms: chrono::DateTime::parse_from_rfc3339(&r.bucket_start)
            .map(|value| value.timestamp_millis())
            .unwrap_or_default(),
        bucket_status: if r.completed_rounds + r.partial_rounds == 0 {
            "skipped".to_owned()
        } else {
            "observed".to_owned()
        },
        rounds_count: r.scheduled_rounds as usize,
        attempted: r.attempted_samples,
        latency_bearing: r.latency_bearing_samples,
        healthy: r.healthy_samples,
        unhealthy: r.unhealthy_samples,
        measurement_lost: r.measurement_loss_samples,
        min_latency_us: r.min_latency_ns.map(|ns| ns / 1_000),
        p50_latency_us: p50,
        p95_latency_us: p95,
        max_latency_us: r.max_latency_ns.map(|ns| ns / 1_000),
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

    let p50 = histogram.quantile(0.5).map(|ns| (ns / 1_000) as i64);
    let p95 = histogram.quantile(0.95).map(|ns| (ns / 1_000) as i64);

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
        timestamp_ms: chrono::DateTime::parse_from_rfc3339(&first.bucket_start)
            .map(|value| value.timestamp_millis())
            .unwrap_or_default(),
        bucket_status: if total_attempted == 0 {
            "skipped".to_owned()
        } else {
            "observed".to_owned()
        },
        rounds_count: total_rounds,
        attempted: total_attempted,
        latency_bearing: total_latency_bearing,
        healthy: total_healthy,
        unhealthy: total_unhealthy,
        measurement_lost: total_measurement_lost,
        min_latency_us: min_lat.map(|ns| ns / 1_000),
        p50_latency_us: p50,
        p95_latency_us: p95,
        max_latency_us: max_lat.map(|ns| ns / 1_000),
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

#[derive(Debug, Clone, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RoundsQuery {
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SampleDetail {
    pub outcome: String,
    pub latency_us: Option<i64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RoundSummary {
    pub timestamp_ms: i64,
    pub execution_status: String,
    pub stop_reason: Option<String>,
    pub attempted_samples: i32,
    pub healthy_samples: i32,
    pub unhealthy_samples: i32,
    pub measurement_loss_samples: i32,
    pub min_latency_us: Option<i64>,
    pub max_latency_us: Option<i64>,
    pub outcome_summary: Option<String>,
    pub samples: Vec<SampleDetail>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RoundsResponse {
    pub rounds: Vec<RoundSummary>,
    pub next_cursor: Option<String>,
}

#[utoipa::path(
    get,
    path = "/api/v1/targets/{target_id}/checks/{check_id}/rounds",
    params(
        ("target_id" = String, Path, description = "Target identifier"),
        ("check_id" = String, Path, description = "Check identifier"),
        RoundsQuery
    ),
    responses(
        (status = 200, body = RoundsResponse),
        (status = 400, body = ApiError),
        (status = 404, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
pub async fn get_rounds(
    State(state): State<AppState>,
    Path((target_id, check_id)): Path<(String, String)>,
    Query(query): Query<RoundsQuery>,
) -> Result<Json<RoundsResponse>, ApiError> {
    let pool = state.pool.clone();
    let observer_id = state.observer_internal_id;
    let limit = validate_limit(query.limit, 50)?;
    let decoded_cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;

    let target = kemuri_storage::TargetRepo::get_by_target_id(&pool, &target_id)
        .await
        .map_err(internal_error)?
        .ok_or_else(|| not_found("target_not_found", "The requested target does not exist"))?;

    let check = kemuri_storage::CheckRepo::get_with_state(
        &pool,
        target.internal_id,
        &check_id,
        observer_id,
    )
    .await
    .map_err(internal_error)?
    .ok_or_else(|| not_found("check_not_found", "The requested check does not exist"))?;

    let rounds = kemuri_storage::RoundRepo::query_recent_by_check(
        &pool,
        check.internal_id,
        observer_id,
        limit + 1,
        decoded_cursor.as_deref(),
    )
    .await
    .map_err(internal_error)?;

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
                timestamp_ms: timestamp_millis(&r.scheduled_at),
                execution_status: r.execution_status.clone(),
                stop_reason: r.stop_reason.clone(),
                attempted_samples: r.attempted_samples,
                healthy_samples: r.healthy_samples,
                unhealthy_samples: r.unhealthy_samples,
                measurement_loss_samples: r.measurement_loss_samples,
                min_latency_us: r.min_latency_ns.map(|ns| ns / 1_000),
                max_latency_us: r.max_latency_ns.map(|ns| ns / 1_000),
                outcome_summary: r.outcome_summary.clone(),
                samples,
            }
        })
        .collect();

    let next_cursor = if has_more {
        rounds
            .last()
            .map(|r| hex::encode(r.scheduled_at.as_bytes()))
    } else {
        None
    };

    Ok(Json(RoundsResponse {
        rounds: summaries,
        next_cursor,
    }))
}

fn validate_limit(value: Option<i64>, default: i64) -> Result<i64, ApiError> {
    let limit = value.unwrap_or(default);
    if !(1..=200).contains(&limit) {
        return Err(bad_request("limit must be between 1 and 200"));
    }
    Ok(limit)
}

fn decode_cursor(cursor: &str) -> Result<String, ApiError> {
    let bytes = hex::decode(cursor).map_err(|_| bad_request("invalid cursor"))?;
    String::from_utf8(bytes).map_err(|_| bad_request("invalid cursor"))
}

fn decode_numeric_cursor(cursor: Option<&str>) -> Result<Option<i64>, ApiError> {
    cursor
        .map(decode_cursor)
        .transpose()?
        .map(|value| value.parse().map_err(|_| bad_request("invalid cursor")))
        .transpose()
}

fn page_by_numeric_id<T>(
    values: &mut Vec<T>,
    limit: i64,
    id: impl Fn(&T) -> i64,
) -> Option<String> {
    let has_more = values.len() > limit as usize;
    values.truncate(limit as usize);
    has_more.then(|| {
        hex::encode(
            values
                .last()
                .map(|value| id(value).to_string())
                .unwrap_or_default(),
        )
    })
}

fn timestamp_millis(value: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.timestamp_millis())
        .unwrap_or_default()
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
                latency_us: rec.latency_ns.map(|ns| (ns / 1_000) as i64),
                metadata,
            }
        })
        .collect()
}

#[utoipa::path(
    get,
    path = "/api/v1/groups",
    params(PageQuery),
    responses(
        (status = 200, body = GroupsResponse),
        (status = 400, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
pub async fn list_groups(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
) -> Result<Json<GroupsResponse>, ApiError> {
    let limit = validate_limit(query.limit, 100)?;
    let cursor = query.cursor.as_deref().map(decode_cursor).transpose()?;
    let all_targets = list_targets(
        State(state),
        Query(PageQuery {
            limit: Some(200),
            cursor: None,
        }),
    )
    .await?
    .0;
    let mut groups = all_targets.groups;
    groups.sort_by(|left, right| left.group_path.cmp(&right.group_path));
    if let Some(cursor) = cursor {
        groups.retain(|group| group.group_path > cursor);
    }
    let has_more = groups.len() > limit as usize;
    groups.truncate(limit as usize);
    let next_cursor = has_more.then(|| {
        hex::encode(
            groups
                .last()
                .map(|group| group.group_path.as_bytes())
                .unwrap_or_default(),
        )
    });
    Ok(Json(GroupsResponse {
        groups,
        next_cursor,
    }))
}

#[utoipa::path(
    get,
    path = "/api/v1/groups/{group_path}",
    params(("group_path" = String, Path, description = "Nested group path")),
    responses(
        (status = 200, body = GroupResponse),
        (status = 404, body = ApiError),
        (status = 500, body = ApiError)
    )
)]
pub async fn get_group(
    State(state): State<AppState>,
    Path(group_path): Path<String>,
) -> Result<Json<GroupResponse>, ApiError> {
    let groups = list_groups(
        State(state),
        Query(PageQuery {
            limit: Some(200),
            cursor: None,
        }),
    )
    .await?
    .0
    .groups;
    let decoded = group_path.trim_matches('/');
    groups
        .into_iter()
        .find(|group| group.group_path == decoded)
        .map(Json)
        .ok_or_else(|| not_found("group_not_found", "The requested group does not exist"))
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GroupsResponse {
    pub groups: Vec<GroupResponse>,
    pub next_cursor: Option<String>,
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
        assert_eq!(merged.min_latency_us, Some(1_000));
        assert_eq!(merged.max_latency_us, Some(20_000));
    }

    #[test]
    fn series_response_has_metadata() {
        let bin_reps_ns = kemuri_core::Histogram::bin_representatives();
        let bin_reps_us: Vec<u64> = bin_reps_ns.iter().map(|&ns| ns / 1_000).collect();
        assert_eq!(bin_reps_us.len(), kemuri_core::Histogram::num_bins());
        assert!(bin_reps_us.last().copied().unwrap_or_default() > 0);
    }

    #[test]
    fn numeric_cursor_pages_in_descending_order() {
        let mut values = vec![9_i64, 8, 7];
        let cursor = page_by_numeric_id(&mut values, 2, |value| *value);
        assert_eq!(values, vec![9, 8]);
        assert_eq!(decode_numeric_cursor(cursor.as_deref()).unwrap(), Some(8));
    }

    #[test]
    fn collection_limit_is_bounded() {
        assert_eq!(validate_limit(Some(1), 50).unwrap(), 1);
        assert_eq!(validate_limit(Some(200), 50).unwrap(), 200);
        assert!(validate_limit(Some(0), 50).is_err());
        assert!(validate_limit(Some(201), 50).is_err());
    }
}
