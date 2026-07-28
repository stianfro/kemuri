import React from 'react';
import { Link } from 'react-router-dom';
import { fetchTargets, fetchAlerts, type TargetsResponse, type GroupResponse, type TargetSummary } from '../api';

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

function StatCard({ label, value, color }: { label: string; value: string | number; color?: string }) {
  return (
    <div
      style={{
        padding: 16,
        background: 'var(--bg-card)',
        border: '1px solid var(--border)',
        borderRadius: 8,
        textAlign: 'center',
      }}
    >
      <div style={{ fontSize: 12, color: 'var(--text-muted)', marginBottom: 4 }}>{label}</div>
      <div style={{ fontSize: 24, fontWeight: 700, color: color || 'var(--text)' }}>{value}</div>
    </div>
  );
}

export function Overview() {
  const [data, setData] = React.useState<TargetsResponse | null>(null);
  const [alertCount, setAlertCount] = React.useState<number>(0);
  const [error, setError] = React.useState<string | null>(null);

  React.useEffect(() => {
    fetchTargets()
      .then(setData)
      .catch((e) => setError(e.message));
    fetchAlerts({ state: 'firing,pending_fire' })
      .then((r) => setAlertCount(r.alerts.length))
      .catch(() => {});
  }, []);

  if (error) return <div style={{ color: 'var(--danger)' }}>Error: {error}</div>;
  if (!data) return <div>Loading targets...</div>;

  if (data.targets.length === 0) {
    return <div>No targets configured. Add targets to your kemuri.yaml configuration.</div>;
  }

  const healthyCount = data.targets.filter((t) => t.state === 'healthy').length;
  const degradedCount = data.targets.filter((t) => t.state === 'degraded').length;
  const downCount = data.targets.filter((t) => t.state === 'down').length;
  const noDataCount = data.targets.filter((t) => t.state === 'no_data').length;

  const worstGroups = data.groups
    .filter((g) => g.targets.some((t) => t.state === 'down' || t.state === 'degraded'))
    .sort((a, b) => {
      const score = (g: GroupResponse) =>
        g.targets.filter((t) => t.state === 'down').length * 3 +
        g.targets.filter((t) => t.state === 'degraded').length;
      return score(b) - score(a);
    });

  return (
    <div>
      <h2 style={{ marginTop: 0 }}>Overview</h2>

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(120px, 1fr))',
          gap: 12,
          marginBottom: 24,
        }}
      >
        <StatCard label="Targets" value={data.targets.length} />
        <StatCard label="Healthy" value={healthyCount} color="var(--success)" />
        <StatCard label="Degraded" value={degradedCount} color="var(--warning)" />
        <StatCard label="Down" value={downCount} color="var(--danger)" />
        <StatCard label="No Data" value={noDataCount} color="var(--text-muted)" />
        <StatCard
          label="Active Alerts"
          value={alertCount}
          color={alertCount > 0 ? 'var(--danger)' : 'var(--success)'}
        />
      </div>

      {alertCount > 0 && (
        <Link
          to="/alerts"
          style={{
            display: 'inline-block',
            padding: '8px 16px',
            marginBottom: 16,
            background: 'rgba(239,68,68,0.1)',
            border: '1px solid rgba(239,68,68,0.3)',
            borderRadius: 6,
            color: 'var(--danger)',
            textDecoration: 'none',
            fontWeight: 600,
            fontSize: 14,
          }}
        >
          {alertCount} active alert{alertCount !== 1 ? 's' : ''}
        </Link>
      )}

      {worstGroups.length > 0 && (
        <div style={{ marginBottom: 24 }}>
          <h3 style={{ color: 'var(--warning)' }}>Attention Needed</h3>
          <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
            {worstGroups.map((g) => (
              <span
                key={g.group_path}
                style={{
                  padding: '4px 10px',
                  background: 'rgba(245,158,11,0.1)',
                  border: '1px solid rgba(245,158,11,0.3)',
                  borderRadius: 6,
                  fontSize: 13,
                  color: 'var(--warning)',
                }}
              >
                {g.group_path}
              </span>
            ))}
          </div>
        </div>
      )}

      {data.groups.map((group: GroupResponse) => (
        <div key={group.group_path} style={{ marginBottom: 24 }}>
          <h3
            style={{
              borderBottom: '1px solid var(--border)',
              paddingBottom: 8,
              fontSize: 16,
            }}
          >
            {group.group_path}
          </h3>
          <table
            style={{
              width: '100%',
              borderCollapse: 'collapse',
              fontSize: 14,
            }}
          >
            <thead>
              <tr style={{ textAlign: 'left', borderBottom: '1px solid var(--border)' }}>
                <th style={{ padding: 8 }}>Target</th>
                <th style={{ padding: 8 }}>State</th>
              </tr>
            </thead>
            <tbody>
              {group.targets.map((t: TargetSummary) => (
                <tr key={t.target_id} style={{ borderBottom: '1px solid var(--border)' }}>
                  <td style={{ padding: 8 }}>
                    <Link
                      to={`/targets/${t.target_id}`}
                      style={{ color: 'var(--accent)', textDecoration: 'none' }}
                    >
                      {t.name || t.target_id}
                    </Link>
                  </td>
                  <td style={{ padding: 8 }}>{stateBadge(t.state)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ))}
    </div>
  );
}
