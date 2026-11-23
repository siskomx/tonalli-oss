export async function loadInstalledMaps(csrf, fetcher = globalThis.fetch) {
  const response = await fetcher('/api/maps', {
    credentials: 'same-origin', cache: 'no-store',
    headers: { Accept: 'application/json', 'X-Tonalli-Csrf': csrf },
  });
  if (!response.ok) throw new Error(`Installed map request failed (${response.status})`);
  const result = await response.json();
  if (!Array.isArray(result.packs)) throw new Error('Invalid installed map inventory');
  return result.packs;
}
