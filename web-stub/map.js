import branding from '../branding.json';
import { loadSession } from './session.js';
import { createTranslator } from './src/locales/index.js';
import { mountMap } from './src/features/map/controller.js';
import './map.css';

new PerformanceObserver((entries) => {
  for (const entry of entries.getEntries()) console.info('MAP_RESOURCE', JSON.stringify({ url: entry.name, type: entry.initiatorType, status: entry.responseStatus }));
}).observe({ type: 'resource', buffered: true });
document.addEventListener('securitypolicyviolation', (event) => console.error('MAP_CSP_BLOCKED', event.violatedDirective, event.blockedURI));
document.title = `${branding.displayName} — Offline map`;
document.getElementById('app-name').textContent = branding.displayName;
const status = document.getElementById('map-status');
const t = createTranslator('en');
loadSession().then((csrf) => mountMap({ container: document.getElementById('map'), selection: document.getElementById('pack-select'),
  language: document.getElementById('language-select'), status, csrf, t }))
  .catch(() => { status.textContent = t('map.failed'); });
