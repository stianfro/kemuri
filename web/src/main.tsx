import { createRoot } from 'react-dom/client';
import { BrowserRouter, Routes, Route, Link, useLocation } from 'react-router-dom';
import { Overview } from './pages/Overview';
import { Target } from './pages/Target';
import { Check } from './pages/Check';
import { Alerts } from './pages/Alerts';
import { System } from './pages/System';
import { fetchAlerts } from './api';
import { useEffect, useState } from 'react';

function NavBar() {
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
        padding: '0 16px',
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
          marginRight: 24,
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
              padding: '12px 16px',
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
    </nav>
  );
}

function App() {
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
        <NavBar />
        <div style={{ maxWidth: 960, margin: '0 auto', padding: '24px 16px' }}>
          <Routes>
            <Route path="/" element={<Overview />} />
            <Route path="/targets/:targetId" element={<Target />} />
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
