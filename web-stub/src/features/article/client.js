const validPack = (value) => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);

export function articleDocumentKind(article){
  const mime=article.mime_type?.split(';',1)[0].trim().toLowerCase();
  if(mime==='application/pdf')return 'pdf';
  if(['application/epub+zip','application/vnd.comicbook+zip','application/x-cbz'].includes(mime))return 'book';
  if(['application/octet-stream','application/zip'].includes(mime)){
    if(/\.pdf$/i.test(article.url))return 'pdf';
    if(/\.(epub|cbz)$/i.test(article.url))return 'book';
  }
  return null;
}

export async function loadArticleMetadata({ pack, url }, csrf, fetcher = globalThis.fetch) {
  if (typeof url !== 'string' || !url || (pack !== undefined && !validPack(pack))) throw new Error('Invalid article selection');
  const query = new URLSearchParams({ url });
  if (pack !== undefined) query.set('pack', pack);
  const response = await fetcher(`/api/zim/article?${query}`, { credentials: 'same-origin', cache: 'no-store',
    headers: { Accept: 'application/json', 'X-Tonalli-Csrf': csrf } });
  if (!response.ok) throw new Error(`Article metadata unavailable (${response.status})`);
  const article = await response.json();
  if (!validPack(article.pack) || !['title', 'url', 'mime_type', 'iframe_url'].every((key) => typeof article[key] === 'string')) {
    throw new Error('Invalid article metadata');
  }
  return article;
}

export function articleFrameUrl(article, origin) {
  if (!validPack(article.pack)) throw new Error('Invalid article pack');
  const target = new URL(article.iframe_url, origin);
  if (target.origin !== origin || target.username || target.password || !target.pathname.startsWith(`/zim/${article.pack}/`)) {
    throw new Error('Article frame URL is outside its installed pack');
  }
  return target.href;
}

export function resolveArticleLink(href, current, pack) {
  try {
    if (!href || !validPack(pack)) return { kind: 'offline' };
    const base = new URL(current);
    const target = new URL(href, base);
    const prefix = `/zim/${pack}/`;
    if (target.origin !== base.origin || !target.pathname.startsWith(prefix)) return { kind: 'offline' };
    if (target.pathname === base.pathname && target.search === base.search && target.hash) return { kind: 'fragment', hash: target.hash };
    const url = decodeURIComponent(target.pathname.slice(prefix.length));
    return url ? { kind: 'internal', url, hash: target.hash } : { kind: 'offline' };
  } catch { return { kind: 'offline' }; }
}
