export function createMapStyle(base, packs, credit = '© OpenStreetMap · Protomaps') {
  const style = structuredClone(base);
  style.sources = Object.create(null);
  style.layers = [];
  for (const pack of packs) {
    // MapLibre 4.7.1 also indexes ordinary objects by source ID; prefix valid pack IDs.
    const sourceId = `pack:${pack.id}`;
    if (!/^[A-Za-z0-9_-]{1,128}$/.test(pack.id) || Object.hasOwn(style.sources, sourceId)) {
      throw new Error('Invalid or duplicate installed map ID');
    }
    style.sources[sourceId] = {
      type: 'vector', url: `pmtiles:///packs/${pack.id}.pmtiles`,
      bounds: pack.bounds,
      minzoom: pack.min_zoom, maxzoom: pack.max_zoom,
      attribution: `<a href="/assets/map/attribution">${escapeHtml(credit)}</a>`
        + (pack.attribution ? ` · ${escapeHtml(attributionText(pack.attribution))}` : ''),
    };
  }
  for (const layer of base.layers) {
    if (!layer.source) style.layers.push(structuredClone(layer));
    else for (const pack of packs) {
      style.layers.push({ ...structuredClone(layer), id: `pack:${pack.id}:${layer.id}`, source: `pack:${pack.id}` });
    }
  }
  return style;
}

function escapeHtml(text) {
  return String(text).replace(/[&<>"']/g, (character) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[character]);
}

function attributionText(value) {
  // Every search starts after consumed input; unfinished tags/bodies are scanned once.
  const input = String(value);
  const chunks = [];
  const whitespace = (character) => character !== undefined && ' \t\r\n\f'.includes(character);
  let position = 0;
  let hidden = null;
  while (position < input.length) {
    if (hidden) {
      const close = input.indexOf('</', position);
      if (close === -1) break;
      const afterName = close + 2 + hidden.length;
      if (input.slice(close + 2, afterName).toLowerCase() === hidden
        && (input[afterName] === '>' || whitespace(input[afterName]))) {
        const end = input.indexOf('>', afterName);
        if (end === -1) break;
        position = end + 1;
        hidden = null;
      } else position = close + 2;
      continue;
    }
    const start = input.indexOf('<', position);
    if (start === -1) { chunks.push(input.slice(position)); break; }
    chunks.push(input.slice(position, start));
    const end = input.indexOf('>', start + 1);
    if (end === -1) { chunks.push(input.slice(start)); break; }
    for (const name of ['script', 'style']) {
      const afterName = start + 1 + name.length;
      if (input.slice(start + 1, afterName).toLowerCase() === name
        && (input[afterName] === '>' || input[afterName] === '/' || whitespace(input[afterName]))) hidden = name;
    }
    position = end + 1;
  }
  const entities = { amp: '&', lt: '<', gt: '>', quot: '"', apos: "'", copy: '©', nbsp: ' ' };
  // Entity matches have a fixed maximum length; decoded text is still escaped by the caller.
  return chunks.join('')
    .replace(/&(#x[0-9a-f]{1,6}|#[0-9]{1,7}|amp|lt|gt|quot|apos|copy|nbsp);/gi, (match, entity) => {
      if (!entity.startsWith('#')) return entities[entity.toLowerCase()];
      const hex = entity[1].toLowerCase() === 'x';
      const codepoint = Number.parseInt(entity.slice(hex ? 2 : 1), hex ? 16 : 10);
      return codepoint > 0 && codepoint <= 0x10ffff && !(codepoint >= 0xd800 && codepoint <= 0xdfff)
        ? String.fromCodePoint(codepoint) : match;
    });
}


export function mapViewFor(packs, selected = packs[0]?.id) {
  if (packs.length === 0) return null;
  const pack = packs.find((candidate) => candidate.id === selected);
  if (!pack) throw new Error('Map is not installed');
  return { center: pack.center, zoom: Math.min(8, pack.max_zoom), maxZoom: pack.max_zoom };
}
