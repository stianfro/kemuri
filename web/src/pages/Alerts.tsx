import React from 'react';
import { Link } from 'react-router-dom';
import {
  fetchAlerts,
  fetchAlertEvents,
  type AlertState,
  type AlertEvent,
} from '../api';

function stateColor(state: string): string {
  const colors: Record<string, string> = {
    firing: 'var(--danger)',
    pending_fire: 'var(--warning)',
    pending_clear: 'var(--accent)',
    normal: 'var(--success)',
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

function formatTime(iso: string | null): string {
  if (!iso) return '-';
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function formatDuration(iso: string | null): string {
  if (!iso) return '-';
  try {
    const start = new Date(iso).getTime();
    const now = Date.now();
    const diffMs = now - start;
    const diffMins = Math.floor(diffMs / 60000);
    if (diffMins < 60) return `${diffMins}m`;
    const diffHours = Math.floor(diffMins / 60);
    if (diffHours < 24) return `${diffHours}h ${diffMins % 60}m`;
    const diffDays = Math.floor(diffHours / 24);
    return `${diffDays}d ${diffHours % 24}h`;
  } catch {
    return '-';
  }
}

export function Alerts() {
  const [alerts, setAlerts] = React.useState<AlertState[]>([]);
  const [events, setEvents] = React.useState<AlertEvent[]>([]);
  const [stateFilter, setStateFilter] = React.useState<string>('firing,pending_fire');
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    fetchAlerts({ state: stateFilter || undefined })
      .then((r) => setAlerts(r.alerts))
      .catch((e) => setError(e.message));
    fetchAlertEvents({ limit: 50 })
      .then((r) => setEvents(r.events))
      .catch(() => {});
  }, [stateFilter]);

  if (error) return <div style={{ color: 'var(--danger)' }}>Error: {error}</div>;

  const activeAlerts = alerts.filter(
    (a) => a.state === 'firing' || a.state === 'pending_fire',
  );

  return (
    <div>
      <h2 style={{ marginTop: 0 }}>Alerts</h2>

      <div style={{ marginBottom: 16, display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        {[
          { label: 'Active', value: 'firing,pending_fire' },
          { label: 'Firing', value: 'firing' },
          { label: 'Pending', value: 'pending_fire,pending_clear' },
          { label: 'All', value: '' },
        ].map((opt) => (
          <button
            key={opt.value}
            onClick={() => setStateFilter(opt.value)}
            style={{
              padding: '4px 12px',
              border: '1px solid var(--border)',
              borderRadius: 6,
              background: stateFilter === opt.value ? 'var(--accent)' : 'var(--bg-card)',
              color: stateFilter === opt.value ? '#fff' : 'var(--text)',
              cursor: 'pointer',
              fontSize: 13,
            }}
          >
            {opt.label}
          </button>
        ))}
      </div>

      {activeAlerts.length > 0 && (
        <>
          <h3>Active Alerts ({activeAlerts.length})</h3>
          <div style={{ overflowX: 'auto' }}>
            <table
              style={{
                width: '100%',
                borderCollapse: 'collapse',
                fontSize: 14,
                marginBottom: 24,
              }}
            >
              <thead>
                <tr style={{ textAlign: 'left', borderBottom: '2px solid var(--danger)' }}>
                  <th style={{ padding: 8 }}>Target</th>
                  <th style={{ padding: 8 }}>Check</th>
                  <th style={{ padding: 8 }}>Rule</th>
                  <th style={{ padding: 8 }}>State</th>
                  <th style={{ padding: 8 }}>Value</th>
                  <th style={{ padding: 8 }}>Duration</th>
                </tr>
              </thead>
              <tbody>
                {activeAlerts.map((a) => (
                  <tr
                    key={a.internal_id}
                    style={{ borderBottom: '1px solid var(--border)' }}
                  >
                    <td style={{ padding: 8 }}>
                      <Link
                        to={`/targets/${a.target_id}`}
                        style={{ color: 'var(--accent)', textDecoration: 'none' }}
                      >
                        {a.target_id}
                      </Link>
                    </td>
                    <td style={{ padding: 8 }}>
                      <Link
                        to={`/targets/${a.target_id}/checks/${a.check_id}`}
                        style={{ color: 'var(--accent)', textDecoration: 'none' }}
                      >
                        {a.check_id}
                      </Link>
                    </td>
                    <td style={{ padding: 8 }}>{a.rule_id}</td>
                    <td style={{ padding: 8 }}>{stateBadge(a.state)}</td>
                    <td style={{ padding: 8 }}>
                      {a.last_metric_value !== null
                        ? (a.last_metric_value * 100).toFixed(1) + '%'
                        : '-'}
                    </td>
                    <td style={{ padding: 8 }}>
                      {formatDuration(a.state_entered_at)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}

      <h3>Alert History</h3>
      {events.length === 0 ? (
        <p style={{ color: 'var(--text-muted)' }}>No alert events recorded.</p>
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
                <th style={{ padding: 6 }}>Type</th>
                <th style={{ padding: 6 }}>Rule</th>
                <th style={{ padding: 6 }}>Target</th>
                <th style={{ padding: 6 }}>Check</th>
                <th style={{ padding: 6 }}>Transition</th>
                <th style={{ padding: 6 }}>Value</th>
              </tr>
            </thead>
            <tbody>
              {events.map((e) => (
                <tr
                  key={e.internal_id}
                  style={{ borderBottom: '1px solid var(--border)' }}
                >
                  <td style={{ padding: 6 }}>{formatTime(e.occurred_at)}</td>
                  <td style={{ padding: 6 }}>
                    {e.event_type === 'firing' ? (
                      <span style={{ color: 'var(--danger)', fontWeight: 600 }}>Firing</span>
                    ) : (
                      <span style={{ color: 'var(--success)', fontWeight: 600 }}>Resolved</span>
                    )}
                  </td>
                  <td style={{ padding: 6 }}>{e.rule_id}</td>
                  <td style={{ padding: 6 }}>
                    <Link
                      to={`/targets/${e.target_id}`}
                      style={{ color: 'var(--accent)', textDecoration: 'none' }}
                    >
                      {e.target_id}
                    </Link>
                  </td>
                  <td style={{ padding: 6 }}>{e.check_id}</td>
                  <td style={{ padding: 6 }}>
                    {e.from_state} &rarr; {e.to_state}
                  </td>
                  <td style={{ padding: 6 }}>
                    {e.metric_value !== null
                      ? (e.metric_value * 100).toFixed(1) + '%'
                      : '-'}
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
