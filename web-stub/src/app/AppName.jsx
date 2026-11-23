import React from 'react';
import branding from '../../../branding.json';

export const displayName = branding.displayName;

export default function AppName() {
  return <span data-testid="app-name">{displayName}</span>;
}
