import { afterEach, expect, test, vi } from 'vitest';

afterEach(() => vi.useRealTimers());

test('perf helper records routeSwitchMs', async () => {
  const { startRouteSwitch } = await import('../../src/app/performance.js');
  vi.useFakeTimers({ toFake: ['performance', 'setTimeout', 'clearTimeout'] });
  const metrics = { bundleKb: 123 };
  const finish = startRouteSwitch(metrics);
  expect(metrics.routeSwitchMs).toBeNull();
  vi.advanceTimersByTime(37);
  finish();
  expect(metrics.routeSwitchMs).toBe(37);
  expect(metrics.bundleKb).toBe(123);
  vi.advanceTimersByTime(11);
  finish();
  expect(metrics.routeSwitchMs).toBe(37);

  const next = startRouteSwitch(metrics);
  expect(metrics.routeSwitchMs).toBeNull();
  vi.advanceTimersByTime(12);
  next();
  expect(metrics.routeSwitchMs).toBe(12);
});
