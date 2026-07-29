import React from 'react';
import { Link, useParams } from '../router';
import { fetchTarget, type TargetDetail, type CheckSummary } from '../api';
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

export function Target() {
  const { targetId } = useParams<{ targetId: string }>();
  const [data, setData] = React.useState<TargetDetail | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const liveRevision = useLiveRefresh();

  React.useEffect(() => {
    if (!targetId) return;
    fetchTarget(targetId)
      .then(setData)
      .catch((e) => setError(e.message));
  }, [targetId, liveRevision]);

  if (error) return <div style={{ color: 'var(--danger)' }}>Error: {error}</div>;
  if (!data) return <div>Loading target...</div>;

  return (
    <div>
      <div style={{ marginBottom: 16, fontSize: 13, color: 'var(--text-muted)' }}>
        <Link to="/" style={{ color: 'var(--accent)', textDecoration: 'none' }}>
          Overview
        </Link>
        {' / '}
        <span>{data.name || data.target_id}</span>
      </div>
      <h2 style={{ marginTop: 0 }}>
        {data.name || data.target_id} {stateBadge(data.state)}
      </h2>
      <div style={{ fontSize: 14, color: 'var(--text-muted)', marginBottom: 16 }}>
        ID: <strong>{data.target_id}</strong> | Group: <strong>{data.group_path || 'default'}</strong>
      </div>

      {Object.keys(data.labels).length > 0 && (
        <div style={{ marginBottom: 16, fontSize: 13 }}>
          {Object.entries(data.labels).map(([k, v]) => (
            <span
              key={k}
              style={{
                display: 'inline-block',
                padding: '2px 8px',
                marginRight: 6,
                marginBottom: 4,
                background: 'var(--bg-card)',
                border: '1px solid var(--border)',
                borderRadius: 4,
              }}
            >
              {k}: {v}
            </span>
          ))}
        </div>
      )}

      <h3 style={{ marginTop: 24 }}>Checks</h3>
      {data.checks.length === 0 ? (
        <p>No checks configured.</p>
      ) : (
        <div style={{ overflowX: 'auto' }}>
          <table
            style={{
              width: '100%',
              borderCollapse: 'collapse',
              fontSize: 14,
            }}
          >
            <thead>
              <tr style={{ textAlign: 'left', borderBottom: '1px solid var(--border)' }}>
                <th style={{ padding: 8 }}>Check</th>
                <th style={{ padding: 8 }}>Type</th>
                <th style={{ padding: 8 }}>State</th>
                <th style={{ padding: 8 }}>Latency</th>
                <th style={{ padding: 8 }}>Loss</th>
              </tr>
            </thead>
            <tbody>
              {data.checks.map((c: CheckSummary) => (
                <tr
                  key={c.check_id}
                  style={{ borderBottom: '1px solid var(--border)' }}
                >
                  <td style={{ padding: 8 }}>
                    <Link
                      to={`/targets/${data.target_id}/checks/${c.check_id}`}
                      style={{ color: 'var(--accent)', textDecoration: 'none' }}
                    >
                      {c.check_id}
                    </Link>
                  </td>
                  <td style={{ padding: 8 }}>{c.probe_type}</td>
                  <td style={{ padding: 8 }}>{stateBadge(c.state)}</td>
                  <td style={{ padding: 8 }}>
                    {formatLatency(c.last_latency_ms)}
                  </td>
                  <td style={{ padding: 8 }}>
                    {c.measurement_loss_ratio !== null
                      ? `${(c.measurement_loss_ratio * 100).toFixed(1)}%`
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
