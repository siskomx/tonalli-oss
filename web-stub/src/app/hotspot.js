// Standard Wi-Fi join QR payload; these characters are escaped by the format.
const escapeField = value => String(value).replace(/([\\;,:"])/g, '\\$1');

export function wifiQrPayload({ ssid, password }) {
  return `WIFI:T:WPA;S:${escapeField(ssid)};P:${escapeField(password)};;`;
}

export const isHotspotNetwork = id => id === 'hotspot' || id === 'hotspot-permission';

// The iPhone cannot create a hotspot or read its name and password; the user switches it on in
// Settings. The native side lists it, labelled "Personal Hotspot", once iOS has it running.
export const personalHotspotNetwork = network => network?.label === 'Personal Hotspot';

// off: not running yet; ready: running, can be chosen; joining: sharing runs on it; none: not relevant.
export function personalHotspotStep(networks, running, url) {
  const hotspot = networks.find(personalHotspotNetwork);
  if (running) {
    let host = '';
    try { host = new URL(url).hostname; } catch { /* no address yet */ }
    return hotspot && host === hotspot.id ? 'joining' : 'none';
  }
  return hotspot ? 'ready' : 'off';
}
