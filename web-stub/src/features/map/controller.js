import maplibregl from 'maplibre-gl';
import { Protocol } from 'pmtiles';
import { loadInstalledMaps } from '../../../map-client.js';
import { createMapStyle, mapViewFor } from '../../../map-style.js';
import 'maplibre-gl/dist/maplibre-gl.css';

const protocol = new Protocol();
maplibregl.addProtocol('pmtiles', protocol.tile);

export function mountMap({ container, selection, language, status, locationStatus, csrf, t, importedNames = {}, onReady, onPending, onError, onSelection }) {
  let map;
  let disposed = false;
  let failed = false;
  let packs;
  const fail = (error) => { if (!disposed) { failed = true; status.textContent = t('map.failed'); onError?.(error); } };
  async function style() {
    const response = await fetch(language.value === 'es' ? '/assets/map/style-es.json' : '/assets/map/style.json', { cache: 'no-store', credentials: 'same-origin' });
    if (!response.ok) throw new Error(`map_style_${response.status}`);
    return createMapStyle(await response.json(), packs, t('map.credit'));
  }
  function choose() {
    const address = new URL(location.href);
    address.searchParams.set('pack', selection.value);
    history.replaceState(history.state, '', address);
    onSelection?.(selection.value, packs);
    onPending?.();
    const view = mapViewFor(packs, selection.value);
    map.setMaxZoom(view.maxZoom);
    map.jumpTo(view);
  }
  async function changeLanguage() {
    onPending?.();
    try { const next = await style(); if (!disposed) map.setStyle(next); }
    catch (error) { fail(error); }
  }
  async function start() {
    packs = await loadInstalledMaps(csrf);
    if (disposed) return;
    selection.replaceChildren(); selection.disabled = false;
    if (!packs.length) {
      status.textContent = t('map.empty'); selection.disabled = true;
      onReady?.({ empty: true, rendered: false, installed: [] }); return;
    }
    for (const pack of packs) {
      const country = /^(?:country-)?(mx|us|ca)(?:-|$)/.exec(pack.id)?.[1];
      selection.add(new Option(country ? t(`map.country.${country}`) : importedNames[pack.id] || t('map.mapFile'), pack.id));
    }
    const requested=new URLSearchParams(location.search).get('pack');
    if(packs.some(pack=>pack.id===requested))selection.value=requested;
    onSelection?.(selection.value, packs);
    const initial = await style();
    if (disposed) return;
    map = new maplibregl.Map({ container, style: initial, ...mapViewFor(packs, selection.value), attributionControl: { compact: false },
      locale: { 'Map.Title': t('nav.map'), 'NavigationControl.ZoomIn': t('map.zoomIn'), 'NavigationControl.ZoomOut': t('map.zoomOut'),
        'NavigationControl.ResetBearing': t('map.resetBearing'), 'AttributionControl.ToggleAttribution': t('map.toggleAttribution'),
        'GeolocateControl.FindMyLocation': t('map.findLocation'), 'GeolocateControl.LocationNotAvailable': t('map.locationUnavailable') },
      transformRequest(url) {
        const archive = url.startsWith('pmtiles://');
        const resolved = new URL(archive ? url.slice(10) : url, location.origin);
        if (resolved.origin !== location.origin) throw new Error('external_map_request');
        return { url: archive ? `pmtiles://${resolved.href}` : resolved.href, credentials: 'same-origin' };
      } });
    map.addControl(new maplibregl.NavigationControl());
    const locate = new maplibregl.GeolocateControl({ positionOptions: { enableHighAccuracy: true, timeout: 20000 }, trackUserLocation: false });
    map.addControl(locate);
    locate.on('geolocate', () => { if (!disposed && locationStatus) locationStatus.textContent = t('map.locationReady'); });
    locate.on('error', event => { if (!disposed && locationStatus) locationStatus.textContent = t(event.code === 1 ? 'map.locationDenied' : 'map.locationUnavailable'); });
    map.on('error', (event) => fail(event.error));
    map.on('idle', () => {
      if (disposed || failed) return;
      const features = map.queryRenderedFeatures().filter((feature) => feature.layer.type === 'symbol');
      const render = { sources: Object.keys(map.getStyle().sources),
        labels: features.filter((feature) => feature.layer.layout?.['text-field']).map((feature) => ({ layer: feature.layer.id,
          name: feature.properties.name, en: feature.properties['name:en'], es: feature.properties['name:es'] })),
        iconLayers: [...new Set(features.filter((feature) => feature.layer.layout?.['icon-image']).map((feature) => feature.layer.id))] };
      container.dataset.renderEvidence = JSON.stringify(render);
      status.textContent = t(packs.length === 1 ? 'map.readyOne' : 'map.readyMany', { count: packs.length, id: selection.value });
      onReady?.({ rendered: true, installed: packs.map((pack) => pack.id), selected: selection.value, ...render });
    });
    selection.addEventListener('change', choose);
    language.addEventListener('change', changeLanguage);
  }
  void start().catch(fail);
  return () => {
    disposed = true;
    selection.removeEventListener('change', choose);
    language.removeEventListener('change', changeLanguage);
    map?.remove();
  };
}
