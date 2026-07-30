import { expect, test, type Page, type Route } from '@playwright/test';

const GROUP_COUNT = 24;
const TARGETS_PER_GROUP = 10;
const TARGET_COUNT = GROUP_COUNT * TARGETS_PER_GROUP;

function stateFor(index: number) {
  return ['healthy', 'degraded', 'down', 'no_data'][index % 4];
}

function groupPath(index: number) {
  return `production/region-${String(index).padStart(2, '0')}/service-${String(index % 6).padStart(2, '0')}`;
}

function target(index: number) {
  return {
    target_id: `target-${String(index).padStart(3, '0')}`,
    name: `Scale target ${String(index).padStart(3, '0')}`,
    group_path: groupPath(Math.floor(index / TARGETS_PER_GROUP)),
    state: stateFor(index),
    checks_count: 1,
  };
}

const targets = Array.from({ length: TARGET_COUNT }, (_, index) => target(index));
const groups = Array.from({ length: GROUP_COUNT }, (_, index) => ({
  group_path: groupPath(index),
  targets: targets.slice(index * TARGETS_PER_GROUP, (index + 1) * TARGETS_PER_GROUP),
}));

function json(route: Route, body: unknown) {
  return route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify(body),
  });
}

function series(targetId: string, checkId: string) {
  const fromMs = 1_700_000_000_000;
  const resolutionMs = 60_000;
  return {
    target_id: targetId,
    check_id: checkId,
    observer_id: 'local',
    from_ms: fromMs,
    to_ms: fromMs + 60 * resolutionMs,
    resolution_ms: resolutionMs,
    source: 'raw',
    quantiles: 'exact',
    histogram_bin_representatives_us: [1_000, 10_000],
    points: Array.from({ length: 60 }, (_, index) => ({
      timestamp_ms: fromMs + index * resolutionMs,
      bucket_status: 'observed',
      rounds_count: 1,
      attempted: 10,
      latency_bearing: 10,
      healthy: 10,
      unhealthy: 0,
      measurement_lost: 0,
      min_latency_us: 900 + index,
      p50_latency_us: 1_000 + index * 10,
      p95_latency_us: 1_500 + index * 10,
      max_latency_us: 2_000 + index * 10,
      measurement_loss_ratio: 0,
      health_failure_ratio: 0,
      histogram_bins: [8, 2],
    })),
    alert_events: [],
    revision_markers: [],
  };
}

async function installScaleApi(page: Page) {
  await page.route('**/api/v1/**', async (route) => {
    const url = new URL(route.request().url());
    const path = url.pathname;
    if (path === '/api/v1/events') {
      await route.fulfill({
        status: 200,
        headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' },
        body: 'retry: 60000\n\n',
      });
      return;
    }
    if (path === '/api/v1/alerts') {
      await json(route, { alerts: [], next_cursor: null });
      return;
    }
    if (path === '/api/v1/targets') {
      await json(route, { targets, groups, next_cursor: null });
      return;
    }
    if (path.startsWith('/api/v1/groups/')) {
      const requested = decodeURIComponent(path.slice('/api/v1/groups/'.length));
      await json(route, groups.find((group) => group.group_path === requested));
      return;
    }
    const seriesMatch = path.match(/^\/api\/v1\/targets\/([^/]+)\/checks\/([^/]+)\/series$/);
    if (seriesMatch) {
      await json(route, series(seriesMatch[1]!, seriesMatch[2]!));
      return;
    }
    const detailMatch = path.match(/^\/api\/v1\/targets\/([^/]+)$/);
    if (detailMatch) {
      const summary = targets.find((item) => item.target_id === detailMatch[1]);
      await json(route, {
        ...summary,
        labels: { environment: 'production' },
        checks: [{
          check_id: 'icmp',
          probe_type: 'icmp',
          state: summary?.state ?? 'no_data',
          last_latency_us: 1_500,
          measurement_loss_ratio: 0,
        }],
      });
      return;
    }
    await route.fulfill({ status: 404, body: `No scale fixture for ${path}` });
  });
}

async function layoutMetrics(page: Page) {
  return page.evaluate(() => ({
    elapsedMs: performance.now(),
    elements: document.querySelectorAll('*').length,
    scrollWidth: document.documentElement.scrollWidth,
    clientWidth: document.documentElement.clientWidth,
  }));
}

test('renders a large nested target inventory within bounded browser resources', async ({ page }, testInfo) => {
  await installScaleApi(page);
  await page.goto('/');
  await expect(page.getByText('Scale target 239')).toBeVisible();

  await expect(page.getByRole('link', { name: /^Scale target / })).toHaveCount(TARGET_COUNT);
  await expect(page.getByRole('link', { name: groupPath(GROUP_COUNT - 1) })).toBeVisible();

  const metrics = await layoutMetrics(page);
  expect(metrics.elapsedMs).toBeLessThan(8_000);
  expect(metrics.elements).toBeLessThan(5_000);
  expect(metrics.scrollWidth).toBeLessThanOrEqual(metrics.clientWidth);
  await page.screenshot({ path: testInfo.outputPath('large-inventory.png'), fullPage: true });
});

test('renders a multi-target group graph without unbounded SVG or DOM growth', async ({ page }, testInfo) => {
  await installScaleApi(page);
  const selectedGroup = groupPath(0);
  await page.goto(`/groups/${encodeURIComponent(selectedGroup)}`);

  const graph = page.getByRole('img', { name: /^10 compared checks\./ });
  await expect(graph).toBeVisible();
  await expect(page.getByText('Scale target 009 / icmp').first()).toBeVisible();
  await expect(graph.locator('path')).toHaveCount(TARGETS_PER_GROUP);

  const metrics = await layoutMetrics(page);
  expect(metrics.elapsedMs).toBeLessThan(8_000);
  expect(metrics.elements).toBeLessThan(2_000);
  expect(metrics.scrollWidth).toBeLessThanOrEqual(metrics.clientWidth);
  await page.screenshot({ path: testInfo.outputPath('group-graph.png'), fullPage: true });
});

test('applies an SSE-driven refresh without reloading the document', async ({ page }) => {
  let targetReads = 0;
  let navigationCount = 0;
  page.on('framenavigated', (frame) => {
    if (frame === page.mainFrame()) navigationCount += 1;
  });

  await page.route('**/api/v1/**', async (route) => {
    const path = new URL(route.request().url()).pathname;
    if (path === '/api/v1/events') {
      await new Promise((resolve) => setTimeout(resolve, 500));
      await route.fulfill({
        status: 200,
        headers: { 'content-type': 'text/event-stream', 'cache-control': 'no-cache' },
        body: 'retry: 60000\nevent: round.completed\ndata: {}\n\n',
      });
      return;
    }
    if (path === '/api/v1/alerts') {
      await json(route, { alerts: [], next_cursor: null });
      return;
    }
    if (path === '/api/v1/targets') {
      targetReads += 1;
      const current = {
        ...target(0),
        name: targetReads === 1 ? 'Before live update' : 'After live update',
      };
      await json(route, {
        targets: [current],
        groups: [{ group_path: current.group_path, targets: [current] }],
        next_cursor: null,
      });
      return;
    }
    await route.fulfill({ status: 404 });
  });

  await page.goto('/');
  await expect(page.getByText('Before live update')).toBeVisible();
  await expect(page.getByText('After live update')).toBeVisible();
  expect(targetReads).toBeGreaterThanOrEqual(2);
  expect(navigationCount).toBe(1);
});
