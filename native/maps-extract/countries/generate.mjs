// Generate the three supported country regions using NOMAD's actual buffering function.
// Usage: node --experimental-strip-types .../generate.mjs path/to/ne_50m_admin_0_countries.geojson
import { readFileSync, writeFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { bufferGeometry } from './buffer.ts';
const raw = readFileSync(process.argv[2]);
const provenance = JSON.parse(readFileSync(new URL('./UPSTREAM.json', import.meta.url)));
if (createHash('sha256').update(raw).digest('hex') !== provenance.geodata_sha256) throw new Error('Natural Earth source differs from the pinned NOMAD dataset');
const source = JSON.parse(raw);
const types = ['Sovereign country','Country','Sovereignty','Disputed','Dependency'];
const rank = feature => { const value=types.indexOf(feature.properties.TYPE);return value<0?5:value; };
const iso = properties => /^[A-Z]{2}$/i.test(properties.ISO_A2 ?? '') ? properties.ISO_A2.toUpperCase() : properties.ISO_A2_EH;
for (const code of ['MX','US','CA']) {
  const original = source.features.filter(feature=>iso(feature.properties)===code).sort((a,b)=>rank(a)-rank(b))[0];
  if (!original) throw new Error('Country missing from source');
  const feature = {type:'Feature',properties:{iso:code,name:original.properties.NAME},geometry:bufferGeometry(original.geometry,.1)};
  writeFileSync(new URL(`./${code.toLowerCase()}.geojson`, import.meta.url), JSON.stringify({type:'FeatureCollection',features:[feature]})+'\n');
}
