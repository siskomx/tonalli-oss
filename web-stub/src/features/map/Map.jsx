import React, { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { useApp, useRouteReadiness } from '../../app/context.js';
import { mountMap } from './controller.js';
import CountryPicker from './CountryPicker.jsx';
import {readingRequest} from '../library/reading-client.js';
import {useQueryClient} from '@tanstack/react-query';
import RemoteCopy from '../library/RemoteCopy.jsx';

export default function MapRoute() {
  const { csrf, t, language, canManageMaps, canConnectRemote, remoteLibrary } = useApp();
  const [copySelection, setCopySelection] = useState({ packs: [], pack: null });
  const [revision, setRevision] = useState(0);
  const [importedNames, setImportedNames] = useState({});
  const readiness = useRouteReadiness('map');
  const callbacks = useRef(readiness);
  const rendered = useRef(null);
  const lastSaved=useRef(null);
  const queryClient=useQueryClient();
  const [historyError,setHistoryError]=useState(false);
  useLayoutEffect(() => { callbacks.current = readiness; }, [readiness]);
  const container = useRef(null);
  const selection = useRef(null);
  const labels = useRef(null);
  const status = useRef(null);
  const locationStatus = useRef(null);
  useEffect(() => {
    rendered.current = null;
    return mountMap({ container: container.current, selection: selection.current, language: labels.current,
      status: status.current, locationStatus: locationStatus.current, csrf, t, importedNames,
      onSelection: (pack, packs) => setCopySelection({ pack, packs }),
      onPending: () => { rendered.current = null; callbacks.current.markPending(); },
      onError: (error) => { rendered.current = null; callbacks.current.markError(error); },
      onReady: (details) => {
        rendered.current = { map: details }; void callbacks.current.markReady(rendered.current);
        if(canManageMaps&&details.selected&&lastSaved.current!==details.selected){lastSaved.current=details.selected;void readingRequest(csrf,'POST',{destination:{kind:'map',pack:details.selected},position:{kind:'start'},title:selection.current?.selectedOptions[0]?.text||t('map.heading'),bookmarked:false}).then(()=>queryClient.invalidateQueries({queryKey:['reading',csrf]})).catch(()=>setHistoryError(true));}
      } });
  }, [csrf, t, revision]);
  useEffect(() => {
    if (rendered.current) void readiness.markReady(rendered.current);
  }, [readiness]);
  return <section>
    <h2 className="text-lg font-semibold">{t('map.heading')}</h2>
    {historyError&&<p role="alert">{t('reading.failed')}</p>}
    {canManageMaps && <CountryPicker onInstalled={imports => {setImportedNames(Object.fromEntries(imports.map(row=>[row.id,row.title])));setRevision(value => value + 1);}} />}
    <div className="my-3 flex flex-wrap gap-4">
      <label>{t('map.pack')} <select ref={selection} id="pack-select" className="rounded border border-slate-400 px-2" /></label>
      <label>{t('map.labels')} <select ref={labels} id="language-select" defaultValue={language} className="rounded border border-slate-400 px-2">
        <option value="en">{t('map.english')}</option><option value="es">{t('map.spanish')}</option>
      </select></label>
    </div>
    {canConnectRemote && remoteLibrary && <RemoteCopy {...copySelection} kind="map" />}
    <p ref={status} id="map-status" role="status" className="my-3">{t('map.loading')}</p>
    <p ref={locationStatus} id="map-location-status" role="status" className="my-3" />
    <a href="/assets/map/attribution" className="text-sm underline">{t('map.credit')}</a>
    <div ref={container} id="map" data-testid="map-renderer" className="mt-3 w-full" style={{ height: '65vh', minHeight: 420 }} />
  </section>;
}
