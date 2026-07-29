export type TimeZoneMode = 'local' | 'utc';

export function formatTime(value: string | number | null): string {
  if (value === null) return '-';
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return String(value);
  const mode = window.localStorage.getItem('kemuri-time-zone');
  return date.toLocaleString(undefined, mode === 'utc' ? { timeZone: 'UTC', timeZoneName: 'short' } : undefined);
}

export function formatAxisTime(value: string | number): string {
  const date = new Date(value);
  const mode = window.localStorage.getItem('kemuri-time-zone');
  return date.toLocaleTimeString(undefined, {
    hour: '2-digit',
    minute: '2-digit',
    ...(mode === 'utc' ? { timeZone: 'UTC' } : {}),
  });
}
