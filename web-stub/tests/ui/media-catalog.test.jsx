import {describe,it,expect} from 'vitest';
import {mediaCategory,mediaHref,searchMedia} from '../../src/features/library/media-catalog.js';
import {readingHref} from '../../src/features/library/reading-client.js';

describe('managed media catalog',()=>{
  it('opens audio and its private history in Music using managed metadata',()=>{
    const file={id:'abc',name:'song.wav'};
    expect(mediaHref(file)).toBe('/music?track=abc');
    expect(readingHref({kind:'media',id:'abc'},[file])).toBe('/music?track=abc');
    expect(readingHref({kind:'media',id:'missing'},[file])).toBe('/files?file=missing');
    expect(mediaHref({id:'abc',name:'song.wav.txt'})).toBe('/file?file=abc');
    expect(mediaHref({id:'abc',name:'photo.png'})).toBe('/gallery?file=abc');
    expect(mediaHref({id:'abc',name:'photo.png.html'})).toBe('/file?file=abc');
    expect(mediaHref({id:'abc',name:'book.PDF'})).toBe('/reader?file=abc');
    expect(readingHref({kind:'media',id:'abc'},[{id:'abc',name:'book.PDF'}])).toBe('/reader?file=abc');
    expect(mediaHref({id:'abc',name:'book.epub'})).toBe('/reader?file=abc');
    expect(mediaHref({id:'abc',name:'comic.cbz'})).toBe('/reader?file=abc');
    expect(mediaHref({id:'abc',name:'Soup.cook'})).toBe('/cookbook?file=abc');
    expect(mediaHref({id:'abc',name:'part.3mf'})).toBe('/workshop?file=abc');
    expect(readingHref({kind:'media',id:'abc'},[{id:'abc',name:'Soup.cook'}])).toBe('/cookbook?file=abc');
  });
  it('recognizes requested categories without treating names as trusted formats',()=>{
    expect(mediaCategory('ALBUM.FLAC')).toBe('music');
    expect(mediaCategory('Chapter.EPUB')).toBe('books');
    expect(mediaCategory('page.cbz')).toBe('comics');
    expect(mediaCategory('part.3mf')).toBe('workshop');
    expect(mediaCategory('dinner.cook')).toBe('recipes');
    expect(mediaCategory('photo.jpg.exe')).toBe('files');
    expect(mediaCategory('movie.mp4')).toBe('video');
    expect(mediaCategory('photo.webp')).toBe('images');
  });
  it('searches names and nested folders with stable identities and category filtering',()=>{
    const rows=[{id:'a',name:'Agua.PDF',folder:'Viajes/México'},
      {id:'b',name:'Agua.FLAC',folder:'Music'},
      {id:'c',name:'Other.txt',folder:'México'}];
    expect(searchMedia(rows,'MÉXICO','all').map(r=>r.id)).toEqual(['a','c']);
    expect(searchMedia(rows,'agua','books').map(r=>r.id)).toEqual(['a']);
    expect(searchMedia(rows,'missing','all')).toEqual([]);
    expect(rows[0]).not.toHaveProperty('category');
  });
});
