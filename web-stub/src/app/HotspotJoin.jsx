import React, { useMemo } from 'react';
import qrcode from 'qrcode-generator';
import { wifiQrPayload } from './hotspot.js';

export default function HotspotJoin({ hotspot, t }) {
  const payload = wifiQrPayload(hotspot);
  const qr = useMemo(() => { const value = qrcode(0, 'M'); value.addData(payload); value.make(); return value; }, [payload]);
  const size = qr.getModuleCount();
  let cells = '';
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if (qr.isDark(y, x)) cells += `M${x+4},${y+4}h1v1h-1z`;
  return <div className="space-y-3" data-testid="hotspot-join">
    <p>{t('sharing.hotspotJoin')}</p>
    <p>{t('sharing.hotspotName')}: <strong className="break-all" data-testid="hotspot-ssid">{hotspot.ssid}</strong></p>
    <p>{t('sharing.hotspotPassword')}: <strong className="break-all" data-testid="hotspot-password">{hotspot.password}</strong></p>
    <svg role="img" aria-label={t('sharing.hotspotQr')} data-testid="hotspot-qr" data-target={payload} viewBox={`0 0 ${size+8} ${size+8}`} width="220" height="220" shapeRendering="crispEdges"><rect width="100%" height="100%" fill="white"/><path d={cells} fill="black"/></svg>
    <p>{t('sharing.hotspotThen')}</p>
  </div>;
}
