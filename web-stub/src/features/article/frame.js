import { articleFrameUrl, loadArticleMetadata, resolveArticleLink, articleDocumentKind } from './client.js';
import { createTranslator } from '../../locales/index.js';

export function attachArticleFrame({ frame, title, status, csrf, t = createTranslator('en'), textSize=100, onReady, onPending, onError, onPosition, onDocument }) {
  const origin = location.origin;
  let metadata = null;
  let activeDocument = null;
  let generation = 0;
  let destroyed = false;
  let lastPosition=null;
  let readingStyle=null;
  function setTextSize(value){
    textSize=[100,115,130,150].includes(value)?value:100;
    if(readingStyle)readingStyle.textContent=`html{font-size:${textSize}%!important}body{font-size:1rem!important;box-sizing:border-box;max-width:80ch;margin-inline:auto!important;padding:12px;overflow-wrap:anywhere}img,video,svg{max-width:100%;height:auto}table{max-width:100%;overflow:auto}pre{white-space:pre-wrap;overflow-wrap:anywhere}`;
  }
  function position(){
    if(!metadata||!activeDocument)return null;
    const view=frame.contentWindow;
    if(view)lastPosition={pack:metadata.pack,url:metadata.url,title:metadata.title,y:view.scrollY};
    return lastPosition?.pack===metadata.pack&&lastPosition.url===metadata.url?lastPosition:null;
  }
  function scrolled(){const value=position();if(value)onPosition?.(value);}

  async function open(pack, url, hash = '') {
    const previous=metadata;
    const previousDocument=activeDocument;
    scrolled();
    activeDocument?.removeEventListener('click',click,true);
    activeDocument?.defaultView?.removeEventListener('scroll',scrolled);
    readingStyle?.remove();readingStyle=null;
    activeDocument=null;
    const request = ++generation;
    metadata = null;
    status.textContent = t('article.loading');
    onPending?.();
    try {
      const result = await loadArticleMetadata({ pack, url }, csrf);
      if (destroyed || request !== generation) return;
      const target = new URL(articleFrameUrl(result, origin));
      target.hash = hash;
      const kind=articleDocumentKind(result);
      if(kind&&onDocument){title.textContent=result.title;status.textContent='';onDocument({...result,kind},previous);return;}
      metadata = result;
      title.textContent = result.title;
      const current=new URL(frame.contentWindow.location.href);
      if(previousDocument&&frame.contentDocument===previousDocument&&target.hash&&current.origin===target.origin&&current.pathname===target.pathname&&current.search===target.search){
        // Fragment navigation has no iframe load event, including resolved aliases.
        frame.contentWindow.location.hash=target.hash;loaded();
      }else frame.src = target.href;
    } catch (error) {
      if (!destroyed && request === generation) { status.textContent = t('article.failed'); onError?.(error); }
    }
  }

  function click(event) {
    const anchor = event.target.closest?.('a,area');
    if (!anchor) return;
    event.preventDefault();
    if (!metadata) return;
    const link = resolveArticleLink(anchor.getAttribute('href') || anchor.getAttributeNS('http://www.w3.org/1999/xlink', 'href'),
      frame.contentWindow.location.href, metadata.pack);
    if (link.kind === 'internal') void open(metadata.pack, link.url, link.hash);
    else if (link.kind === 'fragment') frame.contentWindow.location.hash = link.hash;
    else status.textContent = t('article.offline');
  }

  function loaded() {
    if (!metadata) return;
    try {
      const current = frame.contentWindow.location.href;
      if (current === 'about:blank') return;
      articleFrameUrl({ ...metadata, iframe_url: current }, origin);
      const document = frame.contentDocument;
      const navigation = frame.contentWindow.performance.getEntriesByType('navigation')[0];
      const statusCode = navigation?.responseStatus;
      if (!Number.isInteger(statusCode) || statusCode === 0) throw new Error('article_status_unavailable');
      if (statusCode < 200 || statusCode >= 300) throw new Error(`article_document_${statusCode}`);
      const expectedMime = metadata.mime_type.split(';', 1)[0].trim().toLowerCase();
      if (document.contentType.toLowerCase() !== expectedMime) throw new Error('article_document_mime_mismatch');
      if (activeDocument !== document) {
        activeDocument?.removeEventListener('click', click, true);
        activeDocument?.defaultView?.removeEventListener('scroll',scrolled);
        activeDocument = document;
        if(document.head&&['text/html','application/xhtml+xml'].includes(document.contentType)){
          readingStyle=document.createElement('style');document.head.append(readingStyle);setTextSize(textSize);
        }
        for(const element of document.querySelectorAll('object[data],embed[src],iframe[src]')){
          const href=element.getAttribute('data')||element.getAttribute('src'),link=resolveArticleLink(href,current,metadata.pack);
          if(link.kind!=='internal'||!articleDocumentKind({mime_type:element.getAttribute('type')||'application/octet-stream',url:link.url}))continue;
          const anchor=document.createElement('a');anchor.href=new URL(href,current).href;anchor.textContent=t('article.openDocument');element.replaceWith(anchor);
        }
        for (const anchor of document.querySelectorAll('a,area')) {
          anchor.removeAttribute('ping');
          anchor.removeAttribute('target');
          const href = anchor.getAttribute('href') || anchor.getAttributeNS('http://www.w3.org/1999/xlink', 'href');
          if (resolveArticleLink(href, current, metadata.pack).kind === 'offline') {
            anchor.removeAttribute('href');
            anchor.removeAttributeNS('http://www.w3.org/1999/xlink', 'href');
            anchor.setAttribute('aria-disabled', 'true');
            anchor.insertAdjacentText('afterend', ` (${t('article.marker')})`);
          }
        }
        document.addEventListener('click', click, true);
        document.defaultView.addEventListener('scroll',scrolled,{passive:true});
      }
      status.textContent = t('article.ready');
      onReady?.({ pack: metadata.pack, url: metadata.url, title:metadata.title, frameUrl: current, resourcesLoaded: true });
    } catch (error) {
      activeDocument?.removeEventListener('click', click, true);
      activeDocument?.defaultView?.removeEventListener('scroll',scrolled);
      activeDocument = null;
      status.textContent = t('article.failed');
      onError?.(error);
    }
  }
  frame.addEventListener('load', loaded);
  return { open, position, setTextSize, restore(y){if(Number.isFinite(y)&&y>=0&&activeDocument)frame.contentWindow?.scrollTo(0,y);}, destroy() {
    scrolled();
    destroyed = true;
    generation++;
    frame.removeEventListener('load', loaded);
    activeDocument?.removeEventListener('click', click, true);
    activeDocument?.defaultView?.removeEventListener('scroll',scrolled);
    readingStyle?.remove();readingStyle=null;
  } };
}
