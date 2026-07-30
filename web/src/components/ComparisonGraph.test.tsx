import { render, screen } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';

import type { SeriesResponse } from '../api';
import { ComparisonGraph } from './ComparisonGraph';

function response(targetId: string, medianUs: number): SeriesResponse {
  return {
    target_id: targetId,
    check_id: 'icmp',
    observer_id: 'local',
    from_ms: 0,
    to_ms: 60_000,
    resolution_ms: 30_000,
    source: 'raw',
    quantiles: 'exact',
    histogram_bin_representatives_us: [medianUs],
    points: [
      {
        timestamp_ms: 0,
        bucket_status: 'observed',
        rounds_count: 1,
        attempted: 20,
        latency_bearing: 20,
        healthy: 20,
        unhealthy: 0,
        measurement_lost: 0,
        min_latency_us: medianUs,
        p50_latency_us: medianUs,
        p95_latency_us: medianUs + 500,
        max_latency_us: medianUs + 1_000,
        measurement_loss_ratio: 0,
        health_failure_ratio: 0,
        histogram_bins: [20],
      },
      {
        timestamp_ms: 30_000,
        bucket_status: 'missing',
        rounds_count: 0,
        attempted: 0,
        latency_bearing: 0,
        healthy: 0,
        unhealthy: 0,
        measurement_lost: 0,
        min_latency_us: null,
        p50_latency_us: null,
        p95_latency_us: null,
        max_latency_us: null,
        measurement_loss_ratio: 0,
        health_failure_ratio: 0,
        histogram_bins: [0],
      },
    ],
    alert_events: [],
    revision_markers: [],
  };
}

afterEach(() => {
  vi.restoreAllMocks();
});

it('compares median latency for multiple checks', async () => {
  vi.spyOn(Date, 'now').mockReturnValue(60_000);
  const fetchSeries = vi.fn(
    async (targetId: string) =>
      response(targetId, targetId === 'one' ? 1_000 : 2_000),
  );

  render(
    <ComparisonGraph
      checks={[
        { targetId: 'one', checkId: 'icmp', label: 'One / icmp' },
        { targetId: 'two', checkId: 'icmp', label: 'Two / icmp' },
      ]}
      fetchSeries={fetchSeries}
    />,
  );

  expect(
    await screen.findByLabelText(
      '2 compared checks. One / icmp: 1.0ms; Two / icmp: 2.0ms',
    ),
  ).toBeInTheDocument();
  expect(screen.getAllByText('One / icmp').length).toBeGreaterThan(1);
  expect(screen.getByText('1.5ms')).toBeInTheDocument();
  expect(fetchSeries).toHaveBeenCalledTimes(2);
});

it('reports a comparison request failure', async () => {
  render(
    <ComparisonGraph
      checks={[
        { targetId: 'one', checkId: 'icmp', label: 'One / icmp' },
        { targetId: 'two', checkId: 'icmp', label: 'Two / icmp' },
      ]}
      fetchSeries={vi.fn().mockRejectedValue(new Error('series unavailable'))}
    />,
  );

  expect(await screen.findByRole('alert')).toHaveTextContent('series unavailable');
});
