import React from 'react';
import { Link, useParams } from '../router';
import {
  fetchCheck,
  fetchRounds,
  fetchAlerts,
  fetchSeries,
  type CheckDetail,
  type RoundSummary,
  type AlertState,
} from '../api';
import { SmokeGraph } from '../components/SmokeGraph';
import { formatTime } from '../time';
import { useLiveRefresh } from '../live';

function stateColor(state: string): string {
  const colors: Record<string, string> = {
    healthy: 'var(--success)',
    degraded: 'var(--warning)',
    down: 'var(--danger)',
    no_data: 'var(--text-muted)',
  };
  return colors[state] || 'var(--text-muted)';
}

function stateBadge(state: string) {
  return (
    <span
      style={{
        display: 'inline-block',
        padding: '2px 8px',
        borderRadius: 12,
        fontSize: 12,
        fontWeight: 600,
        color: '#fff',
        backgroundColor: stateColor(state),
      }}
    >
      {state}
    </span>
  );
}

function formatLatency(ms: number | null): string {
  if (ms === null) return '-';
  if (ms < 1) return `${(ms * 1000).toFixed(0)}us`;
  if (ms < 1000) return `${ms.toFixed(1)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

function MetricCard({ label, value }: { label: string; value: string }) {
  return (
    <div
      style={{
        padding: 16,
        background: 'var(--bg-card)',
        border: '1px solid var(--border)',
        borderRadius: 8,
      }}
    >
      <div style={{ fontSize: 12, color: 'var(--text-muted)' }}>{label}</div>
      <div style={{ fontSize: 20, fontWeight: 600 }}>{value}</div>
    </div>
  );
}

export function Check() {
  const { targetId, checkId } = useParams<{
    targetId: string;
    checkId: string;
  }>();
  const [check, setCheck] = React.useState<CheckDetail | null>(null);
  const [rounds, setRounds] = React.useState<RoundSummary[]>([]);
  const [activeAlerts, setActiveAlerts] = React.useState<AlertState[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  const liveRevision = useLiveRefresh();

  React.useEffect(() => {
    if (!targetId || !checkId) return;
    fetchCheck(targetId, checkId)
      .then(setCheck)
      .catch((e) => setError(e.message));
    fetchRounds(targetId, checkId, 20)
      .then((r) => setRounds(r.rounds))
      .catch(() => {});
    fetchAlerts({ target_id: targetId, check_id: checkId, state: 'firing,pending_fire' })
      .then((r) => setActiveAlerts(r.alerts))
      .catch(() => {});
  }, [targetId, checkId, liveRevision]);

  if (error) return <div style={{ color: 'var(--danger)' }}>Error: {error}</div>;
  if (!check) return <div>Loading check...</div>;

  return (
    <div>
      <div style={{ marginBottom: 16, fontSize: 13, color: 'var(--text-muted)' }}>
        <Link to="/" style={{ color: 'var(--accent)', textDecoration: 'none' }}>
          Overview
        </Link>
        {' / '}
        <Link
          to={`/targets/${targetId}`}
          style={{ color: 'var(--accent)', textDecoration: 'none' }}
        >
          {targetId}
        </Link>
        {' / '}
        <span>{checkId}</span>
      </div>

      <h2 style={{ marginTop: 0 }}>
        {check.check_id} {stateBadge(check.state)}
      </h2>

      <div style={{ fontSize: 14, color: 'var(--text-muted)', marginBottom: 16 }}>
        Probe: <strong>{check.probe_type}</strong>
        {check.last_round_at && <> | Last round: {formatTime(check.last_round_at)}</>}
      </div>

      {activeAlerts.length > 0 && (
        <div
          style={{
            padding: 12,
            background: 'rgba(239,68,68,0.1)',
            border: '1px solid rgba(239,68,68,0.3)',
            borderRadius: 8,
            marginBottom: 16,
          }}
        >
          <strong style={{ color: 'var(--danger)' }}>
            {activeAlerts.length} active alert{activeAlerts.length !== 1 ? 's' : ''}
          </strong>
          <div style={{ marginTop: 4, fontSize: 13 }}>
            {activeAlerts.map((a) => (
              <div key={a.internal_id}>
                Rule: {a.rule_id} | State: {a.state} | Value:{' '}
                {a.last_metric_value !== null ? (a.last_metric_value * 100).toFixed(1) + '%' : '-'}
              </div>
            ))}
          </div>
        </div>
      )}

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(3, 1fr)',
          gap: 12,
          margin: '16px 0',
        }}
      >
        <MetricCard label="Latency" value={formatLatency(check.last_latency_ms)} />
        <MetricCard
          label="Measurement Loss"
          value={
            check.measurement_loss_ratio !== null
              ? `${(check.measurement_loss_ratio * 100).toFixed(1)}%`
              : '-'
          }
        />
        <MetricCard
          label="Health Failure"
          value={
            check.health_failure_ratio !== null
              ? `${(check.health_failure_ratio * 100).toFixed(1)}%`
              : '-'
          }
        />
      </div>

      <h3>Smoke Graph</h3>
      {targetId && checkId && (
        <SmokeGraph
          targetId={targetId}
          checkId={checkId}
          fetchSeries={fetchSeries}
        />
      )}

      <h3 style={{ marginTop: 24 }}>Recent Rounds</h3>
      {rounds.length === 0 ? (
        <p style={{ color: 'var(--text-muted)' }}>No rounds recorded yet.</p>
      ) : (
        <div style={{ overflowX: 'auto' }}>
          <table
            style={{
              width: '100%',
              borderCollapse: 'collapse',
              fontSize: 13,
            }}
          >
            <thead>
              <tr style={{ textAlign: 'left', borderBottom: '1px solid var(--border)' }}>
                <th style={{ padding: 6 }}>Time</th>
                <th style={{ padding: 6 }}>Status</th>
                <th style={{ padding: 6 }}>Healthy</th>
                <th style={{ padding: 6 }}>Unhealthy</th>
                <th style={{ padding: 6 }}>Loss</th>
                <th style={{ padding: 6 }}>Min</th>
                <th style={{ padding: 6 }}>Max</th>
              </tr>
            </thead>
            <tbody>
              {rounds.map((r: RoundSummary, i: number) => (
                <tr
                  key={i}
                  style={{ borderBottom: '1px solid var(--border)' }}
                >
                  <td style={{ padding: 6 }}>{formatTime(r.scheduled_at)}</td>
                  <td style={{ padding: 6 }}>{r.execution_status}</td>
                  <td style={{ padding: 6 }}>{r.healthy_samples}</td>
                  <td style={{ padding: 6 }}>{r.unhealthy_samples}</td>
                  <td style={{ padding: 6 }}>{r.measurement_loss_samples}</td>
                  <td style={{ padding: 6 }}>
                    {formatLatency(r.min_latency_ms)}
                  </td>
                  <td style={{ padding: 6 }}>
                    {formatLatency(r.max_latency_ms)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
