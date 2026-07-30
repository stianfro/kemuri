import { useCallback, useEffect, useMemo, useState } from 'react';

import type { SeriesPoint, SeriesResponse } from '../api';
import { formatAxisTime } from '../time';
import { useLiveRefresh } from '../live';

const WIDTH = 900;
const HEIGHT = 320;
const PADDING = { top: 20, right: 20, bottom: 42, left: 64 };
const COLORS = ['#2563eb', '#dc2626', '#16a34a', '#9333ea', '#ea580c', '#0891b2'];
const TIME_RANGES = [
  { label: '1h', duration: 3_600_000 },
  { label: '6h', duration: 21_600_000 },
  { label: '24h', duration: 86_400_000 },
  { label: '7d', duration: 604_800_000 },
];

export interface ComparisonCheck {
  targetId: string;
  checkId: string;
  label: string;
}

interface ComparisonGraphProps {
  checks: ComparisonCheck[];
  fetchSeries: (
    targetId: string,
    checkId: string,
    fromMs: number,
    toMs: number,
    maxPoints?: number,
  ) => Promise<SeriesResponse>;
}

interface LoadedSeries {
  check: ComparisonCheck;
  data: SeriesResponse;
}

function formatLatencyUs(value: number | null | undefined): string {
  if (value == null) return '-';
  if (value < 1_000) return `${value.toFixed(0)}us`;
  if (value < 1_000_000) return `${(value / 1_000).toFixed(1)}ms`;
  return `${(value / 1_000_000).toFixed(2)}s`;
}

function observedPoints(data: SeriesResponse): SeriesPoint[] {
  return data.points.filter(
    (point) => point.bucket_status === 'observed' && point.p50_latency_us != null,
  );
}

function lineSegments(
  data: SeriesResponse,
  xFor: (timestampMs: number) => number,
  yFor: (latencyUs: number) => number,
): string[] {
  const segments: string[] = [];
  let current: string[] = [];
  for (const point of data.points) {
    if (point.bucket_status !== 'observed' || point.p50_latency_us == null) {
      if (current.length > 0) segments.push(current.join(' '));
      current = [];
      continue;
    }
    const command = current.length === 0 ? 'M' : 'L';
    current.push(`${command}${xFor(point.timestamp_ms).toFixed(2)},${yFor(point.p50_latency_us).toFixed(2)}`);
  }
  if (current.length > 0) segments.push(current.join(' '));
  return segments;
}

export function ComparisonGraph({ checks, fetchSeries }: ComparisonGraphProps) {
  const [series, setSeries] = useState<LoadedSeries[]>([]);
  const [rangeMs, setRangeMs] = useState(3_600_000);
  const [logScale, setLogScale] = useState(true);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const liveRevision = useLiveRefresh();

  const load = useCallback(async () => {
    if (checks.length === 0) {
      setSeries([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    const toMs = Date.now();
    const fromMs = toMs - rangeMs;
    try {
      const responses = await Promise.all(
        checks.map(async (check) => ({
          check,
          data: await fetchSeries(
            check.targetId,
            check.checkId,
            fromMs,
            toMs,
            60,
          ),
        })),
      );
      setSeries(responses);
      setError(null);
    } catch (reason) {
      setError(
        reason instanceof Error ? reason.message : 'Could not load comparison data.',
      );
    } finally {
      setLoading(false);
    }
  }, [checks, fetchSeries, rangeMs, liveRevision]);

  useEffect(() => {
    load();
  }, [load]);

  const chart = useMemo(() => {
    const points = series.flatMap(({ data }) => observedPoints(data));
    if (points.length === 0) return null;
    const values = points.map((point) => point.p50_latency_us!);
    const minimum = Math.max(1, Math.min(...values));
    const maximum = Math.max(minimum + 1, Math.max(...values));
    const fromMs = Math.min(...series.map(({ data }) => data.from_ms));
    const toMs = Math.max(...series.map(({ data }) => data.to_ms));
    const plotWidth = WIDTH - PADDING.left - PADDING.right;
    const plotHeight = HEIGHT - PADDING.top - PADDING.bottom;
    const xFor = (timestampMs: number) =>
      PADDING.left +
      ((timestampMs - fromMs) / Math.max(1, toMs - fromMs)) * plotWidth;
    const yFor = (latencyUs: number) => {
      const ratio = logScale
        ? (Math.log10(Math.max(1, latencyUs)) - Math.log10(minimum)) /
          Math.max(0.0001, Math.log10(maximum) - Math.log10(minimum))
        : (latencyUs - minimum) / Math.max(1, maximum - minimum);
      return PADDING.top + plotHeight * (1 - ratio);
    };
    return { minimum, maximum, fromMs, toMs, plotWidth, plotHeight, xFor, yFor };
  }, [series, logScale]);

  const summary = series
    .map(({ check, data }) => {
      const points = observedPoints(data);
      const latest = points[points.length - 1];
      return `${check.label}: ${formatLatencyUs(latest?.p50_latency_us)}`;
    })
    .join('; ');

  return (
    <section aria-labelledby="comparison-heading">
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          flexWrap: 'wrap',
          gap: 8,
          marginBottom: 8,
        }}
      >
        <h2 id="comparison-heading" style={{ fontSize: 18, margin: 0 }}>
          Latency comparison
        </h2>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
          {TIME_RANGES.map((range) => (
            <button
              key={range.label}
              type="button"
              aria-pressed={rangeMs === range.duration}
              onClick={() => setRangeMs(range.duration)}
              style={{
                padding: '4px 10px',
                border: '1px solid var(--border)',
                borderRadius: 4,
                background:
                  rangeMs === range.duration ? 'var(--accent)' : 'transparent',
                color:
                  rangeMs === range.duration ? '#fff' : 'var(--text-muted)',
              }}
            >
              {range.label}
            </button>
          ))}
          <label style={{ fontSize: 12, color: 'var(--text-muted)' }}>
            <input
              type="checkbox"
              checked={logScale}
              onChange={(event) => setLogScale(event.target.checked)}
            />{' '}
            Log scale
          </label>
        </div>
      </div>

      {loading && <p aria-live="polite">{series.length ? 'Refreshing comparison...' : 'Loading comparison...'}</p>}
      {error && <p role="alert">{error}</p>}
      {!loading && !error && !chart && <p>No observed latency data is available.</p>}

      {chart && (
        <>
          <svg
            viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
            role="img"
            aria-label={`${series.length} compared checks. ${summary}`}
            style={{
              display: 'block',
              width: '100%',
              minWidth: 0,
              background: 'var(--bg-card)',
              border: '1px solid var(--border)',
              borderRadius: 6,
            }}
          >
            {[0, 0.25, 0.5, 0.75, 1].map((ratio) => {
              const y = PADDING.top + chart.plotHeight * ratio;
              const value = logScale
                ? 10 ** (
                    Math.log10(chart.maximum) -
                    ratio *
                      (Math.log10(chart.maximum) - Math.log10(chart.minimum))
                  )
                : chart.maximum - ratio * (chart.maximum - chart.minimum);
              return (
                <g key={ratio}>
                  <line
                    x1={PADDING.left}
                    x2={PADDING.left + chart.plotWidth}
                    y1={y}
                    y2={y}
                    stroke="var(--border)"
                  />
                  <text
                    x={PADDING.left - 8}
                    y={y + 4}
                    textAnchor="end"
                    fill="var(--text-muted)"
                    fontSize="11"
                  >
                    {formatLatencyUs(value)}
                  </text>
                </g>
              );
            })}
            {[0, 0.25, 0.5, 0.75, 1].map((ratio) => {
              const timestamp =
                chart.fromMs + ratio * (chart.toMs - chart.fromMs);
              const x = PADDING.left + ratio * chart.plotWidth;
              return (
                <text
                  key={ratio}
                  x={x}
                  y={HEIGHT - 14}
                  textAnchor={
                    ratio === 0 ? 'start' : ratio === 1 ? 'end' : 'middle'
                  }
                  fill="var(--text-muted)"
                  fontSize="11"
                >
                  {formatAxisTime(timestamp)}
                </text>
              );
            })}
            {series.map(({ check, data }, index) =>
              lineSegments(data, chart.xFor, chart.yFor).map((path, pathIndex) => (
                <path
                  key={`${check.targetId}/${check.checkId}/${pathIndex}`}
                  d={path}
                  fill="none"
                  stroke={COLORS[index % COLORS.length]}
                  strokeWidth="2"
                  vectorEffect="non-scaling-stroke"
                />
              )),
            )}
          </svg>

          <div
            style={{
              display: 'flex',
              flexWrap: 'wrap',
              gap: 12,
              marginTop: 8,
              fontSize: 12,
            }}
          >
            {series.map(({ check }, index) => (
              <span key={`${check.targetId}/${check.checkId}`}>
                <span
                  aria-hidden="true"
                  style={{
                    display: 'inline-block',
                    width: 14,
                    borderTop: `3px solid ${COLORS[index % COLORS.length]}`,
                    marginRight: 5,
                    verticalAlign: 'middle',
                  }}
                />
                {check.label}
              </span>
            ))}
          </div>

          <div style={{ overflowX: 'auto', marginTop: 12 }}>
            <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
              <thead>
                <tr style={{ textAlign: 'left', borderBottom: '1px solid var(--border)' }}>
                  <th style={{ padding: 6 }}>Check</th>
                  <th style={{ padding: 6 }}>Latest median</th>
                  <th style={{ padding: 6 }}>Latest p95</th>
                  <th style={{ padding: 6 }}>Loss</th>
                  <th style={{ padding: 6 }}>Observed buckets</th>
                </tr>
              </thead>
              <tbody>
                {series.map(({ check, data }) => {
                  const points = observedPoints(data);
                  const latest = points[points.length - 1];
                  return (
                    <tr
                      key={`${check.targetId}/${check.checkId}`}
                      style={{ borderBottom: '1px solid var(--border)' }}
                    >
                      <td style={{ padding: 6 }}>{check.label}</td>
                      <td style={{ padding: 6 }}>
                        {formatLatencyUs(latest?.p50_latency_us)}
                      </td>
                      <td style={{ padding: 6 }}>
                        {formatLatencyUs(latest?.p95_latency_us)}
                      </td>
                      <td style={{ padding: 6 }}>
                        {latest
                          ? `${(latest.measurement_loss_ratio * 100).toFixed(1)}%`
                          : '-'}
                      </td>
                      <td style={{ padding: 6 }}>{points.length}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </>
      )}
    </section>
  );
}
