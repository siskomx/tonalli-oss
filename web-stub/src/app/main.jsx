import React from 'react';
import { createRoot } from 'react-dom/client';
import { BrowserRouter } from 'react-router-dom';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import App from './App.jsx';
import './styles.css';

window.__perf = { bundleKb: null, firstRenderMs: null, routeSwitchMs: null, listScrollFps: null };
window.__ready = { route: location.pathname, ready: false, error: null };
const client = new QueryClient({ defaultOptions: { queries: {
  retry: false, refetchOnWindowFocus: false, staleTime: Infinity, gcTime: 5000,
} } });
createRoot(document.getElementById('root')).render(
  <QueryClientProvider client={client}><BrowserRouter><App /></BrowserRouter></QueryClientProvider>,
);
