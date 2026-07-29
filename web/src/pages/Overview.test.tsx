import { render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';

import { Overview } from './Overview';

afterEach(() => {
  vi.unstubAllGlobals();
});

it('shows loading and then an empty configuration state', async () => {
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(
      new Response(
        JSON.stringify({ targets: [], groups: [], next_cursor: null }),
        { status: 200 },
      ),
    )
    .mockResolvedValueOnce(
      new Response(JSON.stringify({ alerts: [], next_cursor: null }), { status: 200 }),
    );
  vi.stubGlobal('fetch', fetch);

  render(<Overview />);
  expect(screen.getByText('Loading targets...')).toBeInTheDocument();
  expect(
    await screen.findByText(/No targets configured/),
  ).toBeInTheDocument();
});

it('shows an API error', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue(
      new Response(JSON.stringify({ message: 'database unavailable' }), {
        status: 503,
      }),
    ),
  );

  render(<Overview />);
  await waitFor(() =>
    expect(screen.getByText(/database unavailable/)).toBeInTheDocument(),
  );
});
