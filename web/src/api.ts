export interface BuildInfo {
  version: string;
  git_hash: string;
  build_timestamp: string;
  target: string;
}

export interface TargetSummary {
  target_id: string;
  name: string;
  group_path: string;
  state: string;
  checks_count: number;
}

export interface GroupResponse {
  group_path: string;
  targets: TargetSummary[];
}

export interface TargetsResponse {
  targets: TargetSummary[];
  groups: GroupResponse[];
}

export interface CheckSummary {
  check_id: string;
  probe_type: string;
  state: string;
  last_latency_ms: number | null;
  measurement_loss_ratio: number | null;
}

export interface TargetDetail {
  target_id: string;
  name: string;
  group_path: string;
  labels: Record<string, string>;
  state: string;
  checks: CheckSummary[];
}

export interface CheckDetail {
  check_id: string;
  target_id: string;
  probe_type: string;
  state: string;
  last_latency_ms: number | null;
  measurement_loss_ratio: number | null;
  health_failure_ratio: number | null;
  last_round_at: string | null;
  observer_id: string;
}

export interface SeriesPoint {
  timestamp: string;
  rounds_count: number;
  attempted: number;
  latency_bearing: number;
  healthy: number;
  unhealthy: number;
  measurement_lost: number;
  min_latency_ms: number | null;
  p50_latency_ms: number | null;
  p95_latency_ms: number | null;
  max_latency_ms: number | null;
  measurement_loss_ratio: number;
  health_failure_ratio: number;
  histogram_bins: number[];
}

export interface SeriesResponse {
  target_id: string;
  check_id: string;
  observer_id: string;
  from: string;
  to: string;
  resolution_ms: number;
  source: string;
  quantiles: string;
  histogram_bin_representatives_ms: number[];
  points: SeriesPoint[];
}

export interface SampleDetail {
  outcome: string;
  latency_ms: number | null;
  metadata: Record<string, string> | null;
}

export interface RoundSummary {
  scheduled_at: string;
  execution_status: string;
  stop_reason: string | null;
  attempted_samples: number;
  healthy_samples: number;
  unhealthy_samples: number;
  measurement_loss_samples: number;
  min_latency_ms: number | null;
  max_latency_ms: number | null;
  outcome_summary: string | null;
  samples: SampleDetail[];
}

export interface RoundsResponse {
  rounds: RoundSummary[];
  next_cursor: string | null;
}

export interface AlertState {
  internal_id: number;
  rule_id: string;
  target_id: string;
  check_id: string;
  state: string;
  state_entered_at: string;
  first_condition_true_at: string | null;
  last_evaluated_at: string | null;
  last_notification_at: string | null;
  last_metric_value: number | null;
}

export interface AlertsListResponse {
  alerts: AlertState[];
}

export interface AlertEvent {
  internal_id: number;
  rule_id: string;
  target_id: string;
  check_id: string;
  event_type: string;
  from_state: string;
  to_state: string;
  metric_value: number | null;
  threshold_value: number | null;
  occurred_at: string;
}

export interface AlertEventsResponse {
  events: AlertEvent[];
}

export interface SystemStatus {
  status: string;
  uptime_seconds: number;
  database_path: string;
  database_size_bytes: number;
  schema_version: string;
  config_generation: string | null;
  notification_outbox_pending: number;
  active_alerts: number;
  last_config_reload: {
    generation: string;
    result: string;
    error: string | null;
    timestamp: string;
  } | null;
}

const BASE = '/api/v1';

async function fetchJson<T>(url: string): Promise<T> {
  const res = await fetch(url);
  if (!res.ok) {
    const body = await res.json().catch(() => ({}));
    throw new Error(body.message || `HTTP ${res.status}`);
  }
  return res.json();
}

export async function fetchTargets(): Promise<TargetsResponse> {
  return fetchJson<TargetsResponse>(`${BASE}/targets`);
}

export async function fetchTarget(targetId: string): Promise<TargetDetail> {
  return fetchJson<TargetDetail>(`${BASE}/targets/${targetId}`);
}

export async function fetchChecks(targetId: string): Promise<CheckSummary[]> {
  return fetchJson<CheckSummary[]>(`${BASE}/targets/${targetId}/checks`);
}

export async function fetchCheck(targetId: string, checkId: string): Promise<CheckDetail> {
  return fetchJson<CheckDetail>(`${BASE}/targets/${targetId}/checks/${checkId}`);
}

export async function fetchSeries(
  targetId: string,
  checkId: string,
  from: string,
  to: string,
  maxPoints?: number,
): Promise<SeriesResponse> {
  const params = new URLSearchParams({ from, to });
  if (maxPoints) params.set('max_points', String(maxPoints));
  return fetchJson<SeriesResponse>(
    `${BASE}/targets/${targetId}/checks/${checkId}/series?${params}`,
  );
}

export async function fetchRounds(
  targetId: string,
  checkId: string,
  limit?: number,
  cursor?: string,
): Promise<RoundsResponse> {
  const params = new URLSearchParams();
  if (limit) params.set('limit', String(limit));
  if (cursor) params.set('cursor', cursor);
  const qs = params.toString();
  const query = qs ? `?${qs}` : '';
  return fetchJson<RoundsResponse>(
    `${BASE}/targets/${targetId}/checks/${checkId}/rounds${query}`,
  );
}

export async function fetchGroups(): Promise<GroupResponse[]> {
  return fetchJson<GroupResponse[]>(`${BASE}/groups`);
}

export async function fetchAlerts(params?: {
  state?: string;
  rule_id?: string;
  target_id?: string;
  check_id?: string;
}): Promise<AlertsListResponse> {
  const qs = params
    ? '?' +
      Object.entries(params)
        .filter(([, v]) => v)
        .map(([k, v]) => `${k}=${encodeURIComponent(v!)}`)
        .join('&')
    : '';
  return fetchJson<AlertsListResponse>(`${BASE}/alerts${qs}`);
}

export async function fetchAlert(alertId: number): Promise<AlertState> {
  return fetchJson<AlertState>(`${BASE}/alerts/${alertId}`);
}

export async function fetchAlertEvents(params?: {
  rule_id?: string;
  target_id?: string;
  check_id?: string;
  from?: string;
  to?: string;
  limit?: number;
}): Promise<AlertEventsResponse> {
  const qs = params
    ? '?' +
      Object.entries(params)
        .filter(([, v]) => v)
        .map(([k, v]) => `${k}=${encodeURIComponent(String(v!))}`)
        .join('&')
    : '';
  return fetchJson<AlertEventsResponse>(`${BASE}/alert-events${qs}`);
}

export async function fetchInfo(): Promise<BuildInfo> {
  return fetchJson<BuildInfo>(`${BASE}/info`);
}

export async function fetchSystemStatus(): Promise<SystemStatus> {
  return fetchJson<SystemStatus>(`${BASE}/system/status`);
}
