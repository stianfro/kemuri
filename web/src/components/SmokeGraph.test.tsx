import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';

import type { SeriesPoint, SeriesResponse } from '../api';
import { SmokeGraph } from './SmokeGraph';

const point: SeriesPoint = {
  timestamp_ms: 1_000,
  bucket_status: 'observed',
  rounds_count: 1,
  attempted: 1,
  latency_bearing: 1,
  healthy: 1,
  unhealthy: 0,
  measurement_lost: 0,
  min_latency_us: 1_000,
  p50_latency_us: 1_000,
  p95_latency_us: 1_000,
  max_latency_us: 1_000,
  measurement_loss_ratio: 0,
  health_failure_ratio: 0,
  histogram_bins: [1],
};

beforeEach(() => {
  vi.stubGlobal(
    'ResizeObserver',
    class {
      observe() {}
      disconnect() {}
    },
  );
  vi.stubGlobal(
    'matchMedia',
    vi.fn().mockReturnValue({ matches: false }),
  );
  vi.spyOn(HTMLElement.prototype, 'getBoundingClientRect').mockReturnValue({
    width: 600,
    height: 300,
    top: 0,
    right: 600,
    bottom: 300,
    left: 0,
    x: 0,
    y: 0,
    toJSON: () => ({}),
  });
  vi.spyOn(HTMLCanvasElement.prototype, 'getContext').mockReturnValue(
    new Proxy(
      {},
      {
        get: () => vi.fn(),
      },
    ) as unknown as CanvasRenderingContext2D,
  );
});

afterEach(() => {
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it('reports observed, skipped, and missing buckets accessibly', async () => {
  const response: SeriesResponse = {
    target_id: 'host',
    check_id: 'health',
    observer_id: 'local',
    from_ms: 0,
    to_ms: 900_000,
    resolution_ms: 300_000,
    source: 'rollup',
    quantiles: 'approximate',
    histogram_bin_representatives_us: [1_000],
    points: [
      point,
      { ...point, timestamp_ms: 301_000, bucket_status: 'skipped' },
      { ...point, timestamp_ms: 601_000, bucket_status: 'missing' },
    ],
    alert_events: [],
    revision_markers: [],
  };

  render(
    <SmokeGraph
      targetId="host"
      checkId="health"
      fetchSeries={vi.fn().mockResolvedValue(response)}
    />,
  );

  expect(
    await screen.findByLabelText(
      '3 time buckets: 1 observed, 1 skipped, and 1 missing.',
    ),
  ).toBeInTheDocument();
  expect(screen.getByText(/5-min rollups/)).toBeInTheDocument();
});

it('shows series request failures', async () => {
  render(
    <SmokeGraph
      targetId="host"
      checkId="health"
      fetchSeries={vi.fn().mockRejectedValue(new Error('series unavailable'))}
    />,
  );

  expect(await screen.findByText('series unavailable')).toBeInTheDocument();
});
