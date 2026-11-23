import React, { useMemo, useState } from 'react';
import qrcode from 'qrcode-generator';

export default function SharingAddress({ status, t }) {
  const url = status.friendly_url || status.url;
  const [copied, setCopied] = useState(false);
  const qr = useMemo(() => { const value = qrcode(0, 'M'); value.addData(url); value.make(); return value; }, [url]);
  const size = qr.getModuleCount();
  let cells = '';
  for (let y = 0; y < size; y++) for (let x = 0; x < size; x++) if (qr.isDark(y, x)) cells += `M${x+4},${y+4}h1v1h-1z`;
  return <div className="space-y-3">
    <p className="break-all"><a className="underline" data-testid="sharing-address" href={url} target="_blank" rel="noreferrer">{url}</a></p>
    <svg role="img" aria-label={t('sharing.qr')} data-testid="sharing-qr" data-target={url} viewBox={`0 0 ${size+8} ${size+8}`} width="220" height="220" shapeRendering="crispEdges"><rect width="100%" height="100%" fill="white"/><path d={cells} fill="black"/></svg>
    <button className="secondary-button" onClick={async () => { try { await navigator.clipboard.writeText(url); setCopied(true); } catch { setCopied(false); window.prompt(t('sharing.copyHelp'), url); } }}>{t(copied ? 'sharing.copied' : 'sharing.copy')}</button>
    <details><summary>{t('sharing.details')}</summary><p className="my-3">{t('sharing.fallbackHelp')}</p><a className="underline break-all" href={status.url}>{status.url}</a></details>
  </div>;
}
