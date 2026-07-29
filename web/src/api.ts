import type { components } from './generated/api';

export type BuildInfo = components['schemas']['ApiBuildInfo'];

export type TargetSummary = components['schemas']['TargetSummary'];
export type GroupResponse = components['schemas']['GroupResponse'];
export type TargetsResponse = components['schemas']['TargetsResponse'];
export type CheckSummary = Omit<components['schemas']['CheckSummary'], 'last_latency_us' | 'measurement_loss_ratio'> & {
  last_latency_us: number | null;
  measurement_loss_ratio: number | null;
};
export type TargetDetail = Omit<components['schemas']['TargetDetail'], 'labels' | 'checks'> & {
  labels: Record<string, string>;
  checks: CheckSummary[];
};
export type CheckDetail = Omit<components['schemas']['CheckDetail'], 'last_latency_us' | 'measurement_loss_ratio' | 'health_failure_ratio' | 'last_round_timestamp_ms'> & {
  last_latency_us: number | null;
  measurement_loss_ratio: number | null;
  health_failure_ratio: number | null;
  last_round_timestamp_ms: number | null;
};
export type SeriesPoint = components['schemas']['SeriesPoint'];
export type SeriesResponse = components['schemas']['SeriesResponse'];

export interface SampleDetail {
  outcome: string;
  latency_us: number | null;
  metadata: Record<string, string> | null;
}

export interface RoundSummary {
  timestamp_ms: number;
  execution_status: string;
  stop_reason: string | null;
  attempted_samples: number;
  healthy_samples: number;
  unhealthy_samples: number;
  measurement_loss_samples: number;
  min_latency_us: number | null;
  max_latency_us: number | null;
  outcome_summary: string | null;
  samples: SampleDetail[];
}

export interface RoundsResponse {
  rounds: RoundSummary[];
  next_cursor: string | null;
}

export type AlertState = Omit<components['schemas']['AlertStateResponse'], 'first_condition_true_ms' | 'last_evaluated_ms' | 'last_notification_ms' | 'last_metric_value'> & {
  first_condition_true_ms: number | null;
  last_evaluated_ms: number | null;
  last_notification_ms: number | null;
  last_metric_value: number | null;
};
export type AlertsListResponse = Omit<components['schemas']['AlertsListResponse'], 'alerts'> & { alerts: AlertState[] };
export type AlertEvent = Omit<components['schemas']['AlertEventResponse'], 'metric_value' | 'threshold_value'> & {
  metric_value: number | null;
  threshold_value: number | null;
};
export type AlertEventsResponse = Omit<components['schemas']['AlertEventsResponse'], 'events'> & { events: AlertEvent[] };

export type ChecksResponse = Omit<components['schemas']['ChecksResponse'], 'checks'> & { checks: CheckSummary[] };
export type GroupsResponse = components['schemas']['GroupsResponse'];

export type SystemStatus = components['schemas']['SystemStatus'];

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

export async function fetchChecks(targetId: string): Promise<ChecksResponse> {
  return fetchJson<ChecksResponse>(`${BASE}/targets/${targetId}/checks`);
}

export async function fetchCheck(targetId: string, checkId: string): Promise<CheckDetail> {
  return fetchJson<CheckDetail>(`${BASE}/targets/${targetId}/checks/${checkId}`);
}

export async function fetchSeries(
  targetId: string,
  checkId: string,
  fromMs: number,
  toMs: number,
  maxPoints?: number,
): Promise<SeriesResponse> {
  const params = new URLSearchParams({ from_ms: String(fromMs), to_ms: String(toMs) });
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
  const response = await fetchJson<GroupsResponse>(`${BASE}/groups`);
  return response.groups;
}

export async function fetchGroup(groupPath: string): Promise<GroupResponse> {
  return fetchJson<GroupResponse>(`${BASE}/groups/${encodeURIComponent(groupPath)}`);
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
  from_ms?: number;
  to_ms?: number;
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
