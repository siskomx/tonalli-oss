export function nearestRank(values, percentile) {
  if (!values.length || values.some((value) => !Number.isFinite(value) || value < 0)
    || !(percentile > 0 && percentile <= 1)) throw new Error('invalid percentile samples');
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.ceil(percentile * sorted.length) - 1];
}

export function routeReady(value, route, expectedTitleCount = 19191219) {
  if (value?.route !== route || value.ready !== true || value.error) return false;
  if (route === '/library') return Number.isSafeInteger(expectedTitleCount) && expectedTitleCount >= 100000
    && value.library?.total === expectedTitleCount && value.library.renderedRows > 0;
  if (route === '/article') return value.article?.resourcesLoaded === true;
  if (route === '/map') return value.map?.rendered === true;
  return false;
}

export function visibleGapCount(commit, { top, height, count = 100000, rowHeight = 40, pack = 'wiki' }) {
  if (!Number.isFinite(top) || top < 0 || !Number.isFinite(height) || height <= 0
    || !Number.isSafeInteger(count) || count < 1 || !Number.isFinite(rowHeight) || rowHeight <= 0
    || top >= count * rowHeight) throw new Error('invalid visible geometry');
  const first = Math.max(0, Math.floor(top / rowHeight));
  const last = Math.min(count - 1, Math.ceil((top + height) / rowHeight) - 1);
  const matches = commit?.pack === pack && commit.visibleLimit === count && commit.rowHeight === rowHeight;
  const loaded = new Set(matches ? commit.rows.filter((row) => row.loaded).map((row) => row.index) : []);
  let missing = 0;
  for (let index = first; index <= last; index++) if (!loaded.has(index)) missing++;
  return missing;
}

export function scrollSummary(samples) {
  if (samples.length < 2) throw new Error('scroll needs at least two samples');
  if (samples.some((sample) => !Number.isFinite(sample.at) || !Number.isSafeInteger(sample.missing) || sample.missing < 0)) throw new Error('invalid frame sample');
  const intervals = samples.slice(1).map((sample, index) => sample.at - samples[index].at);
  if (intervals.some((interval) => !(interval > 0))) throw new Error('non-increasing frame timestamps');
  return { fps: intervals.length * 1000 / (samples.at(-1).at - samples[0].at),
    frameTimeP95: nearestRank(intervals, .95), maxFrameTime: Math.max(...intervals),
    visibleGapMs: intervals.reduce((sum, interval, index) => sum + (samples[index].missing > 0 ? interval : 0), 0),
    gapFrames: samples.filter((sample) => sample.missing > 0).length,
    maxMissingRows: Math.max(...samples.map((sample) => sample.missing)) };
}
