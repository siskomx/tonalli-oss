export function startRouteSwitch(metrics) {
  const startedAt = performance.now();
  metrics.routeSwitchMs = null;
  let finished = false;
  return () => {
    if (finished) return;
    finished = true;
    metrics.routeSwitchMs = performance.now() - startedAt;
  };
}
