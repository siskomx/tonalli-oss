import React from 'react';
import { Link } from 'react-router-dom';

// Plain React elements keep this small presentation boundary directly SSR-testable.
export default function BodyResults({ t, error, pending, data, article }) {
  if (pending) return React.createElement('p', { role: 'status' }, t('app.loading'));
  if (error) {
    const key = error.message === 'index_unavailable' ? 'bodyIndexMissing'
      : error.message === 'media_removed' ? 'bodyMediaRemoved' : 'bodyFailed';
    return React.createElement('p', { role: 'alert' }, t(`library.${key}`));
  }
  if (!data) return React.createElement('p', null, t('library.bodyHint'));
  if (!data.hits.length) return React.createElement('p', { role: 'status' }, t('library.bodyNoResults'));
  return React.createElement('ul', { 'aria-label': t('library.bodyResults') }, data.hits.map(row =>
    React.createElement('li', { key: row.url }, React.createElement(Link, { className: 'block py-2 underline', ...article(row) }, row.title))));
}
