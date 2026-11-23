function prefix(value,origin){
  const url=new URL(value,origin);
  if(url.origin!==origin||url.username||url.password||url.search||url.hash||!/^\/book\/[a-f0-9]{64}\/$/.test(url.pathname))throw Error('invalid_book');
  return url;
}
function member(path){return typeof path==='string'&&path.length>0&&path.length<=4096&&!path.startsWith('/')&&!/[\\:\u0000-\u001f\u007f]/.test(path)&&path.split('/').every(part=>part&&part!=='.'&&part!=='..');}
export function bookResourceUrl(base,path,origin=location.origin){
  const root=prefix(base,origin);if(!member(path))throw Error('invalid_book_member');
  return root.href+path.split('/').map(encodeURIComponent).join('/');
}
export function resolveBookLink(href,current,base,origin=location.origin){
  try{
    if(!href)return{kind:'offline'};const root=prefix(base,origin),previous=new URL(current),target=new URL(href,previous);
    if(target.origin!==origin||target.username||target.password||target.search||!target.pathname.startsWith(root.pathname))return{kind:'offline'};
    const path=decodeURIComponent(target.pathname.slice(root.pathname.length));if(!member(path)||!/\.(xhtml|html|htm|svg)$/i.test(path))return{kind:'offline'};
    if(target.pathname===previous.pathname&&href.includes('#'))return{kind:'fragment',hash:target.hash||'#'};
    return{kind:'internal',path,hash:target.hash};
  }catch{return{kind:'offline'};}
}

// Store a DOM text position, not book text, so reflow can retain the same place.
export function framePosition(frame){
  const view=frame?.contentWindow,doc=frame?.contentDocument,root=doc?.body||doc?.documentElement;
  const value={y:Math.max(0,view?.scrollY||0)};
  try{
    const range=doc?.caretRangeFromPoint?.(Math.min(32,view.innerWidth/2),8);let node=range?.startContainer;
    if(node?.nodeType!==3||node.parentElement?.closest('script,style,noscript'))return value;
    const steps=[];while(node&&node!==root&&steps.length<32){const parent=node.parentNode;if(!parent)return value;steps.unshift(Array.prototype.indexOf.call(parent.childNodes,node));node=parent;}
    if(node===root)return{...value,node:steps,offset:range.startOffset};
  }catch{}
  return value;
}
export function restoreFramePosition(frame,value){
  const view=frame?.contentWindow,doc=frame?.contentDocument;if(!view||!doc)return;
  try{
    if(Array.isArray(value?.node)&&value.node.length<=32&&Number.isInteger(value.offset)&&value.offset>=0){
      let node=doc.body||doc.documentElement;for(const index of value.node){if(!Number.isInteger(index)||index<0||!node)throw Error('invalid_position');node=node.childNodes[index];}
      if(node?.nodeType===3&&value.offset<=node.data.length&&!node.parentElement?.closest('script,style,noscript')){
        const range=doc.createRange();range.setStart(node,value.offset);range.collapse(true);const rectangle=range.getBoundingClientRect();
        if(rectangle.height>0&&Number.isFinite(rectangle.top)){view.scrollTo(0,Math.max(0,view.scrollY+rectangle.top-8));return;}
      }
    }
  }catch{}
  if(Number.isFinite(value?.y)&&value.y>=0&&value.y<=1e9)view.scrollTo(0,value.y);
}
