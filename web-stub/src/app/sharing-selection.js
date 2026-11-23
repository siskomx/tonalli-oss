export const MAX_SHARED_FILES = 1024;

// The whole library is shared unless the person removed an item.
export function sharedSelection(packs, files, hiddenPacks, hiddenMedia) {
  const sharedPacks = packs.filter(pack => !hiddenPacks.includes(pack.id)).map(pack => pack.id);
  const sharedMedia = files.filter(file => !hiddenMedia.includes(file.id)).map(file => file.id);
  const total = packs.length + files.length, shared = sharedPacks.length + sharedMedia.length;
  return { packs: sharedPacks, media: sharedMedia, total, shared, hidden: total - shared, tooMany: sharedMedia.length > MAX_SHARED_FILES };
}
