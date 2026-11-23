import React, { useId, useRef, useState } from 'react';
import classNames from './classNames.js';

export default function Tabs({ tabs, label, initial, value, onChange, className }) {
  const base = useId(), refs = useRef([]);
  const [own, setOwn] = useState(initial);
  const requested = value === undefined ? own : value;
  const current = tabs.find(tab => tab.id === requested) || tabs[0];
  const select = id => { setOwn(id); onChange?.(id); };
  function move(event, index) {
    const last = tabs.length - 1;
    const target = { ArrowRight: index === last ? 0 : index + 1, ArrowLeft: index === 0 ? last : index - 1, Home: 0, End: last }[event.key];
    if (target === undefined) return;
    event.preventDefault(); select(tabs[target].id); refs.current[target]?.focus();
  }
  return <div className={classNames('space-y-4', className)}>
    <div role="tablist" aria-label={label} className="flex flex-wrap gap-2 border-b border-border-default pb-3">
      {tabs.map((tab, index) => <button key={tab.id} ref={node => { refs.current[index] = node; }} type="button" role="tab" id={`${base}-tab-${tab.id}`}
        aria-selected={tab.id === current.id} aria-controls={`${base}-panel-${tab.id}`} tabIndex={tab.id === current.id ? 0 : -1}
        className={classNames('shrink-0', tab.id === current.id ? 'chat-control' : 'secondary-button')}
        onClick={() => select(tab.id)} onKeyDown={event => move(event, index)}>{tab.label}{tab.badge ? <span className="ml-2 text-sm">({tab.badge})</span> : null}</button>)}
    </div>
    <div role="tabpanel" id={`${base}-panel-${current.id}`} aria-labelledby={`${base}-tab-${current.id}`} tabIndex={0} className="space-y-6">{current.content}</div>
  </div>;
}
