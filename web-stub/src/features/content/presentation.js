export function actionsFor(state) {
  if (['queued', 'downloading', 'verifying', 'waiting_wifi'].includes(state)) return ['pause', 'cancel'];
  if (state === 'paused') return ['resume', 'cancel'];
  if (state === 'failed') return ['retry', 'cancel'];
  if (state === 'cancelled') return ['retry'];
  return [];
}
export function progressFor(job) {
  return { value: ['verifying', 'installing'].includes(job.state) ? job.verifiedBytes : job.downloadedBytes,
    max: job.totalBytes, state: job.state };
}
export function projectionFor(budget) {
  return { percent: budget.availableBytes === 0 ? (budget.requiredBytes ? 100 : 0)
    : Math.min(100, budget.requiredBytes / budget.availableBytes * 100),
  remainingBytes: Math.max(0, budget.availableBytes - budget.requiredBytes), allowed: budget.allowed };
}
export function errorKey(code, t) {
  const key = `content.error.${code}`;
  return t(key) === key ? 'content.error.failed' : key;
}
