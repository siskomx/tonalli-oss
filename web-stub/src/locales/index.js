import en from './en.json';
import es from './es.json';

export function resolveLanguage(languages = globalThis.navigator?.languages || []) {
  for (const language of languages) {
    const short = language.toLowerCase().split('-')[0];
    if (short === 'en' || short === 'es') return short;
  }
  return 'en';
}

export function createTranslator(language, catalogs = { en, es }) {
  const selected = catalogs[language] || catalogs.en;
  return (key, values = {}) => (selected[key] ?? catalogs.en[key] ?? key)
    .replace(/\{(\w+)\}/g, (placeholder, name) => String(values[name] ?? placeholder));
}
