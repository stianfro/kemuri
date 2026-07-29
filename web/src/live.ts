import { useEffect, useState } from 'react';

export const LIVE_REFRESH_EVENT = 'kemuri:refresh';

export function requestLiveRefresh() {
  window.dispatchEvent(new Event(LIVE_REFRESH_EVENT));
}

export function useLiveRefresh(): number {
  const [revision, setRevision] = useState(0);

  useEffect(() => {
    const refresh = () => setRevision((current) => current + 1);
    window.addEventListener(LIVE_REFRESH_EVENT, refresh);
    return () => window.removeEventListener(LIVE_REFRESH_EVENT, refresh);
  }, []);

  return revision;
}
