import { afterEach, describe, expect, it } from 'vitest';

import { formatAxisTime, formatTime } from './time';

describe('time formatting', () => {
  afterEach(() => window.localStorage.clear());

  it('formats Unix milliseconds', () => {
    window.localStorage.setItem('kemuri-time-zone', 'utc');
    expect(formatTime(0)).toContain('1970');
    expect(formatAxisTime(0)).toMatch(/00|12/);
  });

  it('returns a placeholder for a missing timestamp', () => {
    expect(formatTime(null)).toBe('-');
  });
});
