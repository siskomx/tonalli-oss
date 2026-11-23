import {it,expect} from 'vitest';
import {videoCatalog} from '../../src/features/video/catalog.js';

it('groups seasons and specials, sorts episodes numerically and keeps covers local',()=>{
  const rows=[
    {id:'movie',name:'A film.mp4',folder:'Movies/A film'},
    {id:'cover',name:'A film.jpg',folder:'Movies/A film'},
    {id:'ten',name:'River.S02E10.mp4',folder:'Shows/River/Season 02'},
    {id:'two',name:'River.S02E02.webm',folder:'Shows/River/Season 02'},
    {id:'special',name:'Bonus.m4v',folder:'Shows/River/Especiales'},
    {id:'poster',name:'POSTER.PNG',folder:'Shows/River'},
    {id:'root',name:'Ocean.S01E01.mp4',folder:''},
    {id:'trap',name:'poster.svg',folder:'Shows/River'},
  ];
  const catalog=videoCatalog(rows);
  expect(catalog.movies.map(row=>row.id)).toEqual(['movie']);expect(catalog.movies[0].poster).toBe('cover');
  const show=catalog.shows.find(show=>show.title==='River');expect(show.poster).toBe('poster');
  expect(show.seasons.map(season=>season.number)).toEqual([2,0]);
  expect(show.seasons[0].episodes.map(episode=>episode.id)).toEqual(['two','ten']);
  expect(show.seasons[1].episodes[0].poster).toBe('poster');
  expect(catalog.shows.find(show=>show.title==='Ocean').seasons[0].number).toBe(1);
  const filtered=videoCatalog(rows,'E02');expect(filtered.shows[0].seasons[0].episodes[0].poster).toBe('poster');
  expect(rows[0]).not.toHaveProperty('poster');
});
it('preserves folder identities and does not treat ordinary numbered filenames as episodes',()=>{
  const rows=[{id:'a',name:'2026 Vacation.mp4',folder:'Trips'},
    {id:'b',name:'One.mp4',folder:'Shows/__proto__/Temporada 01'},
    {id:'c',name:'Two.mp4',folder:'Shows/__proto__/Season 02'}];
  const catalog=videoCatalog(rows);expect(catalog.movies[0].id).toBe('a');
  expect(catalog.shows[0].title).toBe('__proto__');expect(catalog.shows[0].seasons.map(season=>season.number)).toEqual([1,2]);
});
