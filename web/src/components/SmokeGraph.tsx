import React, { useRef, useEffect, useState, useCallback } from 'react';
import type { SeriesPoint, SeriesResponse } from '../api';
import { formatTime } from '../time';

const STRIP_HEIGHT = 16;
const PADDING = { top: 20, right: 20, bottom: 40, left: 60 };
const TIME_RANGES = [
  { label: '1h', duration: 3600000 },
  { label: '6h', duration: 21600000 },
  { label: '24h', duration: 86400000 },
  { label: '7d', duration: 604800000 },
  { label: '30d', duration: 2592000000 },
  { label: '90d', duration: 7776000000 },
];

interface SmokeGraphProps {
  targetId: string;
  checkId: string;
  fetchSeries: (targetId: string, checkId: string, from: string, to: string, maxPoints?: number) => Promise<SeriesResponse>;
}

interface TooltipData {
  point: SeriesPoint;
  x: number;
  y: number;
}

function formatLatency(ms: number): string {
  if (ms < 1) return `${(ms * 1000).toFixed(0)}us`;
  if (ms < 1000) return `${ms.toFixed(1)}ms`;
  return `${(ms / 1000).toFixed(2)}s`;
}

export function SmokeGraph({ targetId, checkId, fetchSeries }: SmokeGraphProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);
  const [data, setData] = useState<SeriesResponse | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [logScale, setLogScale] = useState(true);
  const [rangeMs, setRangeMs] = useState(3600000);
  const [tooltip, setTooltip] = useState<TooltipData | null>(null);
  const [canvasSize, setCanvasSize] = useState({ width: 800, height: 400 });

  const loadData = useCallback(async () => {
    const to = new Date();
    const from = new Date(to.getTime() - rangeMs);
    const maxPoints = Math.max(100, Math.floor(canvasSize.width / 2));
    try {
      const result = await fetchSeries(
        targetId,
        checkId,
        from.toISOString(),
        to.toISOString(),
        maxPoints,
      );
      setData(result);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Failed to load series data');
    }
  }, [targetId, checkId, rangeMs, canvasSize.width, fetchSeries]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const observer = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width } = entry.contentRect;
        setCanvasSize({
          width: Math.max(400, Math.floor(width)),
          height: Math.max(300, Math.floor(width * 0.5)),
        });
      }
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    if (!data || !canvasRef.current) return;

    const canvas = canvasRef.current;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    canvas.width = canvasSize.width * dpr;
    canvas.height = canvasSize.height * dpr;
    canvas.style.width = `${canvasSize.width}px`;
    canvas.style.height = `${canvasSize.height}px`;
    ctx.scale(dpr, dpr);

    const plotW = canvasSize.width - PADDING.left - PADDING.right;
    const plotH = canvasSize.height - PADDING.top - PADDING.bottom - STRIP_HEIGHT * 2;
    const graphH = plotH;

    ctx.clearRect(0, 0, canvasSize.width, canvasSize.height);

    const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches;
    const bgColor = isDark ? '#1a1a2e' : '#f0f1f5';
    const gridColor = isDark ? '#2a2a4a' : '#d1d5db';
    const textColor = isDark ? '#9ca3af' : '#6b7280';

    ctx.fillStyle = bgColor;
    ctx.fillRect(0, 0, canvasSize.width, canvasSize.height);

    const points = data.points;
    if (points.length === 0) {
      ctx.fillStyle = textColor;
      ctx.font = '14px system-ui';
      ctx.textAlign = 'center';
      ctx.fillText('No data for this time range', canvasSize.width / 2, canvasSize.height / 2);
      return;
    }

    const binReps = data.histogram_bin_representatives_ms;
    const numBins = binReps.length;

    let minLatMs = Infinity;
    let maxLatMs = -Infinity;
    for (const rep of binReps) {
      if (rep > 0) {
        minLatMs = Math.min(minLatMs, rep);
        maxLatMs = Math.max(maxLatMs, rep);
      }
    }
    if (!isFinite(minLatMs)) {
      minLatMs = 0.001;
      maxLatMs = 1000;
    }

    const yToPixel = (latMs: number): number => {
      if (logScale) {
        const logMin = Math.log10(Math.max(minLatMs, 0.001));
        const logMax = Math.log10(maxLatMs);
        const logVal = Math.log10(Math.max(latMs, 0.001));
        const ratio = (logVal - logMin) / (logMax - logMin || 1);
        return PADDING.top + graphH * (1 - ratio);
      }
      const ratio = latMs / (maxLatMs || 1);
      return PADDING.top + graphH * (1 - ratio);
    };

    const graphTop = PADDING.top;
    const graphBottom = PADDING.top + graphH;
    const lossTop = graphBottom + 2;
    const healthTop = lossTop + STRIP_HEIGHT + 2;

    const cellW = plotW / points.length;

    for (let index = 0; index < points.length; index += 1) {
      const point = points[index]!;
      if (point.bucket_status === 'observed') continue;
      const x = PADDING.left + index * cellW;
      ctx.fillStyle =
        point.bucket_status === 'missing'
          ? 'rgba(107,114,128,0.22)'
          : 'rgba(245,158,11,0.18)';
      ctx.fillRect(x, graphTop, cellW + 0.5, healthTop + STRIP_HEIGHT - graphTop);
      if (point.bucket_status === 'skipped') {
        ctx.strokeStyle = 'rgba(245,158,11,0.45)';
        ctx.lineWidth = 1;
        for (let offset = -graphH; offset < cellW + graphH; offset += 8) {
          ctx.beginPath();
          ctx.moveTo(x + offset, healthTop + STRIP_HEIGHT);
          ctx.lineTo(x + offset + graphH, graphTop);
          ctx.stroke();
        }
      }
    }

    const timeToX = (timestampMs: number) =>
      PADDING.left +
      ((timestampMs - data.from_ms) / Math.max(1, data.to_ms - data.from_ms)) * plotW;
    const firingByRule = new Map<string, number>();
    for (const event of data.alert_events) {
      if (event.event_type === 'firing') {
        firingByRule.set(event.rule_id, event.timestamp_ms);
      } else if (event.event_type === 'resolved') {
        const startedAt = firingByRule.get(event.rule_id);
        if (startedAt !== undefined) {
          const startX = timeToX(startedAt);
          const endX = timeToX(event.timestamp_ms);
          ctx.fillStyle = 'rgba(239,68,68,0.10)';
          ctx.fillRect(startX, graphTop, Math.max(1, endX - startX), graphH);
          firingByRule.delete(event.rule_id);
        }
      }
    }
    for (const startedAt of firingByRule.values()) {
      const startX = timeToX(startedAt);
      ctx.fillStyle = 'rgba(239,68,68,0.10)';
      ctx.fillRect(startX, graphTop, Math.max(1, PADDING.left + plotW - startX), graphH);
    }
    ctx.strokeStyle = 'rgba(59,130,246,0.75)';
    ctx.lineWidth = 1;
    for (const marker of data.revision_markers) {
      const x = timeToX(marker.timestamp_ms);
      ctx.beginPath();
      ctx.moveTo(x, graphTop);
      ctx.lineTo(x, graphBottom);
      ctx.stroke();
    }

    let globalMaxBin = 0;
    for (const p of points) {
      for (let b = 0; b < numBins && b < p.histogram_bins.length; b++) {
        const val = p.histogram_bins[b];
        if (val != null && val > globalMaxBin) {
          globalMaxBin = val;
        }
      }
    }

    for (let i = 0; i < points.length; i++) {
      const x = PADDING.left + i * cellW;
      const bins = points[i]!.histogram_bins;

      for (let b = 0; b < numBins && b < bins.length; b++) {
        const binVal = bins[b];
        if (binVal == null || binVal === 0) continue;
        const latMs = binReps[b];
        if (latMs == null || latMs <= 0) continue;

        const py = yToPixel(latMs);
        const cellH = Math.max(2, graphH / numBins);

        const intensity = globalMaxBin > 0 ? binVal / globalMaxBin : 0;
        const r = Math.floor(50 + intensity * 200);
        const g = Math.floor(80 + intensity * 120);
        const bl = Math.floor(200 - intensity * 150);
        const alpha = 0.2 + intensity * 0.8;

        ctx.fillStyle = `rgba(${r},${g},${bl},${alpha})`;
        ctx.fillRect(x, py - cellH / 2, cellW + 0.5, cellH);
      }
    }

    ctx.strokeStyle = '#22c55e';
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    for (let i = 0; i < points.length; i++) {
      const x = PADDING.left + i * cellW + cellW / 2;
      const p50 = points[i]!.p50_latency_ms;
      if (p50 != null) {
        const y = yToPixel(p50);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
    }
    ctx.stroke();

    ctx.strokeStyle = '#f59e0b';
    ctx.lineWidth = 1;
    ctx.setLineDash([4, 4]);
    ctx.beginPath();
    for (let i = 0; i < points.length; i++) {
      const x = PADDING.left + i * cellW + cellW / 2;
      const p95 = points[i]!.p95_latency_ms;
      if (p95 != null) {
        const y = yToPixel(p95);
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      }
    }
    ctx.stroke();
    ctx.setLineDash([]);

    for (let i = 0; i < points.length; i++) {
      const x = PADDING.left + i * cellW;
      const p = points[i]!;
      const total = p.healthy + p.unhealthy + p.measurement_lost;
      if (total > 0) {
        const lossRatio = p.measurement_lost / total;
        const lossAlpha = Math.min(0.2 + lossRatio * 3, 1);
        ctx.fillStyle = `rgba(239,68,68,${lossAlpha})`;
        ctx.fillRect(x, lossTop, cellW + 0.5, STRIP_HEIGHT);

        if (lossRatio > 0.1) {
          ctx.strokeStyle = 'rgba(239,68,68,0.6)';
          ctx.lineWidth = 0.5;
          for (let s = 0; s < cellW; s += 6) {
            ctx.beginPath();
            ctx.moveTo(x + s, lossTop + STRIP_HEIGHT);
            ctx.lineTo(x + s + STRIP_HEIGHT, lossTop);
            ctx.stroke();
          }
        }
      }

      if (total > 0 && p.unhealthy > 0) {
        const hfRatio = p.unhealthy / total;
        const hfAlpha = Math.min(0.2 + hfRatio * 3, 1);
        ctx.fillStyle = `rgba(245,158,11,${hfAlpha})`;
        ctx.fillRect(x, healthTop, cellW + 0.5, STRIP_HEIGHT);

        if (hfRatio > 0.05) {
          ctx.strokeStyle = 'rgba(245,158,11,0.5)';
          ctx.lineWidth = 0.5;
          for (let s = 0; s < cellW; s += 4) {
            ctx.beginPath();
            ctx.moveTo(x + s, healthTop);
            ctx.lineTo(x + s + STRIP_HEIGHT, healthTop + STRIP_HEIGHT);
            ctx.stroke();
          }
        }
      }
    }

    ctx.fillStyle = textColor;
    ctx.font = '10px system-ui';
    ctx.textAlign = 'center';
    ctx.fillText('Loss', PADDING.left - 30, lossTop + STRIP_HEIGHT / 2 + 3);
    ctx.fillText('Health', PADDING.left - 30, healthTop + STRIP_HEIGHT / 2 + 3);

    ctx.strokeStyle = gridColor;
    ctx.lineWidth = 0.5;
    ctx.strokeRect(PADDING.left, graphTop, plotW, graphH);

    const numYTicks = logScale ? 6 : 5;
    ctx.fillStyle = textColor;
    ctx.font = '11px system-ui';
    ctx.textAlign = 'right';
    for (let i = 0; i <= numYTicks; i++) {
      const ratio = i / numYTicks;
      let latMs: number;
      if (logScale) {
        const logMin = Math.log10(Math.max(minLatMs, 0.001));
        const logMax = Math.log10(maxLatMs);
        latMs = Math.pow(10, logMin + ratio * (logMax - logMin));
      } else {
        latMs = ratio * maxLatMs;
      }
      const y = yToPixel(latMs);
      ctx.fillText(formatLatency(latMs), PADDING.left - 5, y + 4);
      ctx.strokeStyle = gridColor;
      ctx.beginPath();
      ctx.moveTo(PADDING.left, y);
      ctx.lineTo(PADDING.left + plotW, y);
      ctx.stroke();
    }

    ctx.fillStyle = textColor;
    ctx.font = '11px system-ui';
    ctx.textAlign = 'center';
    const xLabelCount = Math.min(8, points.length);
    const xStep = Math.max(1, Math.floor(points.length / xLabelCount));
    for (let i = 0; i < points.length; i += xStep) {
      const x = PADDING.left + i * cellW + cellW / 2;
      const pt = points[i];
      const label = pt ? formatTime(pt.timestamp).split(' ').pop() || '' : '';
      ctx.fillText(label, x, canvasSize.height - PADDING.bottom + 20);
    }
  }, [data, canvasSize, logScale]);

  const handleMouseMove = useCallback(
    (e: React.MouseEvent<HTMLCanvasElement>) => {
      if (!data || data.points.length === 0) return;
      const canvas = canvasRef.current;
      if (!canvas) return;

      const rect = canvas.getBoundingClientRect();
      const mx = e.clientX - rect.left;

      const plotW = canvasSize.width - PADDING.left - PADDING.right;
      const cellW = plotW / data.points.length;
      const idx = Math.floor((mx - PADDING.left) / cellW);

      if (idx >= 0 && idx < data.points.length) {
        const point = data.points[idx]!;
        setTooltip({ point, x: e.clientX - rect.left, y: e.clientY - rect.top });
      } else {
        setTooltip(null);
      }
    },
    [data, canvasSize],
  );

  const handleMouseLeave = useCallback(() => {
    setTooltip(null);
  }, []);

  const resolutionLabel = data
    ? data.resolution_ms === 0
      ? 'Raw'
      : data.resolution_ms <= 300000
        ? '5-min rollups'
        : '1-hour rollups'
    : '';

  const graphSummary = data
    ? `${data.points.length} time buckets: ${
        data.points.filter((point) => point.bucket_status === 'observed').length
      } observed, ${
        data.points.filter((point) => point.bucket_status === 'skipped').length
      } skipped, and ${
        data.points.filter((point) => point.bucket_status === 'missing').length
      } missing.`
    : 'Latency graph is loading.';

  return (
    <div ref={containerRef} style={{ width: '100%', position: 'relative' }}>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          marginBottom: 8,
          flexWrap: 'wrap',
          gap: 8,
        }}
      >
        <div style={{ display: 'flex', gap: 4 }}>
          {TIME_RANGES.map((r) => (
            <button
              key={r.label}
              onClick={() => setRangeMs(r.duration)}
              style={{
                padding: '4px 10px',
                fontSize: 12,
                border: '1px solid',
                borderColor: rangeMs === r.duration ? 'var(--accent)' : 'var(--border)',
                borderRadius: 4,
                background: rangeMs === r.duration ? 'var(--accent)' : 'transparent',
                color: rangeMs === r.duration ? 'var(--accent)' : 'var(--text-muted)',
                cursor: 'pointer',
              }}
            >
              {r.label}
            </button>
          ))}
        </div>
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          {data && (
            <span style={{ fontSize: 11, color: 'var(--text-muted)' }}>
              {resolutionLabel} | {data.quantiles} quantiles | {data.source}
            </span>
          )}
          <label
            style={{
              fontSize: 12,
              color: 'var(--text-muted)',
              display: 'flex',
              alignItems: 'center',
              gap: 4,
              cursor: 'pointer',
            }}
          >
            <input
              type="checkbox"
              checked={logScale}
              onChange={(e) => setLogScale(e.target.checked)}
            />
            Log scale
          </label>
        </div>
      </div>

      {error && <div style={{ color: '#ef4444', fontSize: 13 }}>{error}</div>}

      <div style={{ position: 'relative' }}>
        <canvas
          ref={canvasRef}
          role="img"
          aria-label={graphSummary}
          onMouseMove={handleMouseMove}
          onMouseLeave={handleMouseLeave}
          style={{ display: 'block', borderRadius: 4 }}
        />
        <p
          style={{
            position: 'absolute',
            width: 1,
            height: 1,
            padding: 0,
            margin: -1,
            overflow: 'hidden',
            clip: 'rect(0, 0, 0, 0)',
            whiteSpace: 'nowrap',
            border: 0,
          }}
        >
          {graphSummary}
        </p>

        {tooltip && (
          <div
            style={{
              position: 'absolute',
              left: Math.min(tooltip.x + 10, canvasSize.width - 260),
              top: Math.max(tooltip.y - 180, 5),
              background: 'var(--bg-card)',
              border: '1px solid var(--border)',
              borderRadius: 6,
              padding: 10,
              fontSize: 12,
              color: 'var(--text)',
              pointerEvents: 'none',
              zIndex: 10,
              minWidth: 220,
            }}
          >
            <div style={{ fontWeight: 600, marginBottom: 4 }}>
              {formatTime(tooltip.point.timestamp)}
            </div>
            <div>Rounds: {tooltip.point.rounds_count}</div>
            <div>
              Samples: {tooltip.point.attempted} attempted,{' '}
              {tooltip.point.latency_bearing} latency-bearing
            </div>
            <div>
              Healthy: {tooltip.point.healthy} | Unhealthy:{' '}
              {tooltip.point.unhealthy} | Loss: {tooltip.point.measurement_lost}
            </div>
            <div>
              Loss ratio:{' '}
              {(tooltip.point.measurement_loss_ratio * 100).toFixed(1)}% |
              Health failure:{' '}
              {(tooltip.point.health_failure_ratio * 100).toFixed(1)}%
            </div>
            {tooltip.point.min_latency_ms != null && (
              <div>
                Min: {formatLatency(tooltip.point.min_latency_ms)} | Median:{' '}
                {tooltip.point.p50_latency_ms != null
                  ? formatLatency(tooltip.point.p50_latency_ms)
                  : '-'}
              </div>
            )}
            {tooltip.point.p95_latency_ms != null && (
              <div>
                P95: {formatLatency(tooltip.point.p95_latency_ms)} | Max:{' '}
                {tooltip.point.max_latency_ms != null
                  ? formatLatency(tooltip.point.max_latency_ms)
                  : '-'}
              </div>
            )}
            {data && (
              <div style={{ fontSize: 10, color: 'var(--text-muted)', marginTop: 4 }}>
                {data.quantiles === 'exact' ? 'Exact' : 'Approximate'} values |{' '}
                {data.source}
              </div>
            )}
          </div>
        )}
      </div>

      <div
        style={{
          display: 'flex',
          gap: 16,
          marginTop: 6,
          fontSize: 11,
          color: 'var(--text-muted)',
        }}
      >
        <span>
          <span
            style={{
              display: 'inline-block',
              width: 12,
              height: 3,
              background: '#22c55e',
              marginRight: 4,
              verticalAlign: 'middle',
            }}
          />
          Median
        </span>
        <span>
          <span
            style={{
              display: 'inline-block',
              width: 12,
              height: 0,
              borderTop: '2px dashed #f59e0b',
              marginRight: 4,
              verticalAlign: 'middle',
            }}
          />
          P95
        </span>
        <span>
          <span
            style={{
              display: 'inline-block',
              width: 12,
              height: 8,
              background: 'rgba(239,68,68,0.6)',
              marginRight: 4,
              verticalAlign: 'middle',
            }}
          />
          Measurement loss
        </span>
        <span>
          <span
            style={{
              display: 'inline-block',
              width: 12,
              height: 8,
              background: 'rgba(245,158,11,0.6)',
              marginRight: 4,
              verticalAlign: 'middle',
            }}
          />
          Health failure
        </span>
      </div>
    </div>
  );
}
