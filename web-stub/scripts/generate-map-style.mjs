import { layers, namedFlavor } from '@protomaps/basemaps';
import { writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';

for (const lang of ['en', 'es']) {
  const style = {
    version: 8,
    name: `Protomaps light (${lang})`,
    glyphs: '/assets/map/fonts/{fontstack}/{range}.pbf',
    sprite: '/assets/map/sprites/sprite',
    sources: { protomaps: { type: 'vector', url: 'pmtiles:///packs/installed.pmtiles' } },
    layers: layers('protomaps', namedFlavor('light'), { lang }),
  };
  await writeFile(resolve(process.argv[2], lang === 'en' ? 'style.json' : 'style-es.json'), `${JSON.stringify(style, null, 2)}\n`, 'utf8');
}
