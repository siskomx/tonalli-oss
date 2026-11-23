import React from 'react';
import { personalHotspotStep } from './hotspot.js';

// iPhone only: guides the user through Personal Hotspot, which apps are not allowed to switch on.
export default function PersonalHotspot({ networks, running, url, t }) {
  const step = personalHotspotStep(networks, running, url);
  if (step === 'none') return null;
  return <div className="space-y-3" data-testid="personal-hotspot" data-step={step}>
    {step !== 'joining' && <h4>{t('sharing.iosHotspotTitle')}</h4>}
    {step === 'off' && <><p>{t('sharing.iosHotspotOff')}</p><p>{t('sharing.iosHotspotWait')}</p></>}
    {step === 'ready' && <p>{t('sharing.iosHotspotReady')}</p>}
    {step === 'joining' && <p>{t('sharing.iosHotspotJoin')}</p>}
  </div>;
}
