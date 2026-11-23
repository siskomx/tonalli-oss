// A bounded receipt of committed virtual rows, without content or layout reads.
export function libraryCommit(pack, visibleLimit, rowHeight, items, rowAt) {
  return { pack, visibleLimit, rowHeight,
    rows: items.map(({ index }) => ({ index, loaded: Boolean(rowAt(index)) })) };
}
