const NOTES = {
  copying: 'copy.searchProgress',
  copied: 'copy.searchCopied',
  unavailable: 'copy.searchUnavailable',
  failed: 'copy.searchFailed',
  cancelled: 'copy.searchCancelled',
};

// Only books can bring a prepared article search; maps never do.
export const canCopySearch = kind => kind === 'zim';

export const copyStartBody = (id, kind, withSearch) => (canCopySearch(kind) && withSearch ? { id, search: true } : { id });

export const copySearchNote = copy => (copy?.search && Object.hasOwn(NOTES, copy.search) ? NOTES[copy.search] : null);
