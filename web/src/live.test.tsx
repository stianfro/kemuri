import { act, renderHook } from '@testing-library/react';
import { expect, it } from 'vitest';

import { requestLiveRefresh, useLiveRefresh } from './live';

it('increments the live revision without reloading the page', () => {
  const { result } = renderHook(() => useLiveRefresh());
  expect(result.current).toBe(0);
  act(() => requestLiveRefresh());
  expect(result.current).toBe(1);
});
