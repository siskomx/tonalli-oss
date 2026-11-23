// Adapted from Project NOMAD admin/inertia/components/inputs/Select.tsx at c903da48c97bc02f5cdfeff2ebcde00a82dfec8e.
// Copyright 2024-2026 Crosstalk Solutions LLC. Apache-2.0.
// Modifications: JSX/shared classNames; native select replaces Headless UI Listbox
// and icon dependencies, retaining label/help/error presentation and native keyboard semantics.
import React from 'react';
import classNames from './classNames.js';

export default function Select({ name, label, value, onChange, options, helpText, className,
  labelClassName, selectClassName, containerClassName, error, required, disabled }) {
  return <div className={classNames(className)}>
    <label htmlFor={name} className={classNames('block text-base/6 font-medium text-text-primary', labelClassName)}>{label}{required ? '*' : ''}</label>
    {helpText && <p className="mt-1 text-sm text-text-muted">{helpText}</p>}
    <div className={classNames('mt-1.5', containerClassName)}>
      <select id={name} name={name} value={value} onChange={event => onChange(event.target.value)} disabled={disabled} required={required}
        className={classNames('w-full rounded-md bg-surface-primary px-3 py-2 text-base border border-border-default text-text-primary focus:outline focus:outline-2 focus:-outline-offset-2 focus:outline-primary',
          error && '!border-red-500 focus:outline-red-500 !bg-red-100', disabled && 'opacity-50 cursor-not-allowed', selectClassName)}>
        {options.map(option => <option key={option.value} value={option.value} disabled={option.disabled}>{option.label}</option>)}
      </select>
    </div>
  </div>;
}
