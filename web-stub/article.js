import branding from '../branding.json';
import { loadSession } from './session.js';
import { attachArticleFrame } from './src/features/article/frame.js';
import './src/features/article/article.css';

document.title = `${branding.displayName} — Offline article`;
document.getElementById('app-name').textContent = branding.displayName;
const status = document.getElementById('article-status');
loadSession().then((csrf) => {
  const { open } = attachArticleFrame({ frame: document.getElementById('article-frame'), title: document.getElementById('article-title'), status, csrf });
  const query = new URLSearchParams(location.search);
  const url = query.get('url');
  if (url) void open(query.get('pack') ?? undefined, url);
  else status.textContent = 'Choose an installed article URL';
}).catch((error) => { status.textContent = error.message; });
