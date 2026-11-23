import {describe,it,expect} from 'vitest';
import {bookResourceUrl,resolveBookLink} from '../../src/features/documents/book-client.js';
const origin='http://127.0.0.1:42123',base=`/book/${'a'.repeat(64)}/`;
describe('private book links',()=>{
  it('encodes member names without accepting another origin or namespace',()=>{
    expect(bookResourceUrl(base,'OPS/niñez y educación.xhtml',origin)).toBe(`${origin}${base}OPS/ni%C3%B1ez%20y%20educaci%C3%B3n.xhtml`);
    for(const path of ['../notes.db','/notes.db','C:/notes.db','OPS\\notes.db','OPS/./page.xhtml'])expect(()=>bookResourceUrl(base,path,origin)).toThrow();
    for(const prefix of ['https://example.invalid'+base,base+'?token=x','/api/notes/'])expect(()=>bookResourceUrl(prefix,'page.xhtml',origin)).toThrow();
  });
  it('keeps fragments and chapter links inside this exact archive',()=>{
    const current=bookResourceUrl(base,'OPS/one.xhtml',origin);
    expect(resolveBookLink('#section',current,base,origin)).toEqual({kind:'fragment',hash:'#section'});
    expect(resolveBookLink('../other/two.xhtml#footnote',current,base,origin)).toEqual({kind:'internal',path:'other/two.xhtml',hash:'#footnote'});
    for(const href of ['https://example.invalid/book','javascript:alert(1)','../../../../api/notes','/settings',`/book/${'b'.repeat(64)}/one.xhtml`,'../data.json'])expect(resolveBookLink(href,current,base,origin)).toEqual({kind:'offline'});
  });
});
