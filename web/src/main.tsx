import { createRoot } from 'react-dom/client';
import { BrowserRouter, Routes, Route, Link, useLocation } from './router';
import { Overview } from './pages/Overview';
import { Target } from './pages/Target';
import { Check } from './pages/Check';
import { Alerts } from './pages/Alerts';
import { System } from './pages/System';
import { Group } from './pages/Group';
import { fetchAlerts } from './api';
import { useEffect, useState } from 'react';

function NavBar({
  timeZone,
  onTimeZoneChange,
}: {
  timeZone: 'local' | 'utc';
  onTimeZoneChange: (value: 'local' | 'utc') => void;
}) {
  const location = useLocation();
  const [alertCount, setAlertCount] = useState(0);

  useEffect(() => {
    fetchAlerts({ state: 'firing,pending_fire' })
      .then((r) => setAlertCount(r.alerts.length))
      .catch(() => {});
    const id = setInterval(() => {
      fetchAlerts({ state: 'firing,pending_fire' })
        .then((r) => setAlertCount(r.alerts.length))
        .catch(() => {});
    }, 30000);
    return () => clearInterval(id);
  }, []);

  const links = [
    { to: '/', label: 'Overview' },
    { to: '/alerts', label: 'Alerts', badge: alertCount > 0 ? alertCount : undefined },
    { to: '/system', label: 'System' },
  ];

  return (
    <nav
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 0,
        borderBottom: '1px solid var(--border)',
        padding: '0 8px',
        height: 48,
        background: 'var(--bg-nav)',
        position: 'sticky',
        top: 0,
        zIndex: 100,
      }}
    >
      <Link
        to="/"
        style={{
          fontWeight: 700,
          fontSize: 16,
          color: 'var(--text)',
          textDecoration: 'none',
          marginRight: 8,
        }}
      >
        Kemuri
      </Link>
      {links.map((link) => {
        const active =
          link.to === '/' ? location.pathname === '/' : location.pathname.startsWith(link.to);
        return (
          <Link
            key={link.to}
            to={link.to}
            style={{
              padding: '12px 8px',
              fontSize: 14,
              fontWeight: active ? 600 : 400,
              color: active ? 'var(--text)' : 'var(--text-muted)',
              textDecoration: 'none',
              borderBottom: active ? '2px solid var(--accent)' : '2px solid transparent',
              display: 'flex',
              alignItems: 'center',
              gap: 6,
            }}
          >
            {link.label}
            {link.badge !== undefined && (
              <span
                style={{
                  background: '#ef4444',
                  color: '#fff',
                  fontSize: 11,
                  fontWeight: 700,
                  borderRadius: 10,
                  padding: '1px 6px',
                  minWidth: 18,
                  textAlign: 'center',
                }}
              >
                {link.badge}
              </span>
            )}
          </Link>
        );
      })}
      <button
        type="button"
        aria-label="Toggle local and UTC time"
        onClick={() => onTimeZoneChange(timeZone === 'local' ? 'utc' : 'local')}
        style={{
          marginLeft: 'auto',
          border: '1px solid var(--border)',
          borderRadius: 4,
          padding: '5px 8px',
          background: 'var(--bg-card)',
          color: 'var(--text)',
        }}
      >
        {timeZone === 'local' ? 'Local time' : 'UTC'}
      </button>
    </nav>
  );
}

function SystemEventBridge() {
  useEffect(() => {
    const stream = new EventSource('/api/v1/events');
    let disconnected = false;
    let refresh: number | undefined;
    const scheduleRefresh = () => {
      window.clearTimeout(refresh);
      refresh = window.setTimeout(() => window.location.reload(), 250);
    };
    for (const eventType of [
      'round.completed',
      'check.state_changed',
      'alert.firing',
      'alert.resolved',
      'config.reloaded',
      'system.status_changed',
    ]) {
      stream.addEventListener(eventType, scheduleRefresh);
    }
    stream.onerror = () => {
      disconnected = true;
    };
    stream.onopen = () => {
      if (disconnected) scheduleRefresh();
      disconnected = false;
    };
    return () => {
      window.clearTimeout(refresh);
      stream.close();
    };
  }, []);
  return null;
}

function App() {
  const [timeZone, setTimeZone] = useState<'local' | 'utc'>(() =>
    window.localStorage.getItem('kemuri-time-zone') === 'utc' ? 'utc' : 'local',
  );
  const [theme, setTheme] = useState<'light' | 'dark'>(() => {
    if (typeof window !== 'undefined' && window.matchMedia('(prefers-color-scheme: dark)').matches) {
      return 'dark';
    }
    return 'light';
  });

  useEffect(() => {
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = (e: MediaQueryListEvent) => setTheme(e.matches ? 'dark' : 'light');
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  }, []);
  useEffect(() => {
    window.localStorage.setItem('kemuri-time-zone', timeZone);
  }, [timeZone]);

  const colors =
    theme === 'dark'
      ? {
          '--bg': '#0f0f1a',
          '--bg-nav': '#161625',
          '--bg-card': '#1a1a2e',
          '--bg-hover': '#22223a',
          '--text': '#e5e7eb',
          '--text-muted': '#9ca3af',
          '--border': '#2a2a4a',
          '--accent': '#3b82f6',
          '--success': '#22c55e',
          '--warning': '#f59e0b',
          '--danger': '#ef4444',
        }
      : {
          '--bg': '#f8f9fa',
          '--bg-nav': '#ffffff',
          '--bg-card': '#ffffff',
          '--bg-hover': '#f0f0f5',
          '--text': '#1f2937',
          '--text-muted': '#6b7280',
          '--border': '#e5e7eb',
          '--accent': '#3b82f6',
          '--success': '#16a34a',
          '--warning': '#d97706',
          '--danger': '#dc2626',
        };

  return (
    <div
      style={{
        ...Object.fromEntries(Object.entries(colors).map(([k, v]) => [k, v] as const)),
        minHeight: '100vh',
        background: 'var(--bg)',
        color: 'var(--text)',
        fontFamily: 'system-ui, -apple-system, sans-serif',
      } as React.CSSProperties}
    >
      <BrowserRouter>
        <SystemEventBridge />
        <NavBar timeZone={timeZone} onTimeZoneChange={setTimeZone} />
        <div style={{ maxWidth: 960, margin: '0 auto', padding: '24px 16px' }}>
          <Routes>
            <Route path="/" element={<Overview />} />
            <Route path="/targets/:targetId" element={<Target />} />
            <Route path="/groups/:groupPath" element={<Group />} />
            <Route path="/targets/:targetId/checks/:checkId" element={<Check />} />
            <Route path="/alerts" element={<Alerts />} />
            <Route path="/system" element={<System />} />
          </Routes>
        </div>
      </BrowserRouter>
    </div>
  );
}

const root = createRoot(document.getElementById('root')!);
root.render(<App />);
