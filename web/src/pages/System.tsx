import React from 'react';
import { fetchInfo, fetchSystemStatus, type BuildInfo, type SystemStatus } from '../api';
import { useLiveRefresh } from '../live';
import { formatTime } from '../time';

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatUptime(seconds: number): string {
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  const parts: string[] = [];
  if (days > 0) parts.push(`${days}d`);
  if (hours > 0) parts.push(`${hours}h`);
  parts.push(`${mins}m`);
  return parts.join(' ');
}

function Card({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div
      style={{
        background: 'var(--bg-card)',
        border: '1px solid var(--border)',
        borderRadius: 8,
        padding: 16,
        marginBottom: 16,
      }}
    >
      <h3
        style={{
          margin: '0 0 12px 0',
          fontSize: 14,
          fontWeight: 600,
          color: 'var(--text-muted)',
          textTransform: 'uppercase',
          letterSpacing: 0.5,
        }}
      >
        {title}
      </h3>
      {children}
    </div>
  );
}

function Row({ label, value }: { label: string; value: string | React.ReactNode }) {
  return (
    <div
      style={{
        display: 'flex',
        justifyContent: 'space-between',
        padding: '6px 0',
        borderBottom: '1px solid var(--border)',
        fontSize: 14,
      }}
    >
      <span style={{ color: 'var(--text-muted)' }}>{label}</span>
      <span style={{ fontWeight: 500 }}>{value}</span>
    </div>
  );
}

export function System() {
  const [info, setInfo] = React.useState<BuildInfo | null>(null);
  const [status, setStatus] = React.useState<SystemStatus | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const liveRevision = useLiveRefresh();

  React.useEffect(() => {
    fetchInfo().then(setInfo).catch((e) => setError(e.message));
    fetchSystemStatus().then(setStatus).catch((e) => setError(e.message));
    const id = setInterval(() => {
      fetchSystemStatus().then(setStatus).catch(() => {});
    }, 10000);
    return () => clearInterval(id);
  }, [liveRevision]);

  if (error) return <div style={{ color: 'var(--danger)' }}>Error: {error}</div>;
  if (!info || !status) return <div>Loading system info...</div>;

  return (
    <div>
      <h2 style={{ marginTop: 0 }}>System Status</h2>

      <Card title="Version">
        <Row label="Version" value={info.version} />
        <Row label="Git Hash" value={info.git_hash} />
        <Row label="Build Target" value={info.target} />
        <Row label="Built At" value={formatTime(info.build_timestamp_ms)} />
      </Card>

      <Card title="Runtime">
        <Row label="Status" value={status.status} />
        <Row label="Uptime" value={formatUptime(status.uptime_seconds)} />
        <Row label="Active Alerts" value={String(status.active_alerts)} />
        <Row label="Notification Backlog" value={String(status.notification_outbox_pending)} />
      </Card>

      <Card title="Database">
        <Row label="Path" value={status.database_path} />
        <Row label="Size" value={formatBytes(status.database_size_bytes)} />
        <Row label="Schema Version" value={status.schema_version} />
      </Card>

      <Card title="Configuration">
        <Row
          label="Generation"
          value={status.config_generation || '-'}
        />
        <Row
          label="Last Reload"
          value={
            status.last_config_reload
              ? formatTime(status.last_config_reload.timestamp_ms)
              : 'Never'
          }
        />
        {status.last_config_reload && (
          <>
            <Row
              label="Reload Result"
              value={
                <span
                  style={{
                    color:
                      status.last_config_reload.result === 'success'
                        ? 'var(--success)'
                        : 'var(--danger)',
                  }}
                >
                  {status.last_config_reload.result}
                </span>
              }
            />
            {status.last_config_reload.error && (
              <Row label="Reload Error" value={status.last_config_reload.error} />
            )}
          </>
        )}
      </Card>
    </div>
  );
}
