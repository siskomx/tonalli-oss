import {searchMedia} from '../library/media-catalog.js';

const stem=name=>name.replace(/\.[^.]+$/,'');
const title=name=>stem(name).replace(/[._]+/g,' ').trim();
const order=(a,b)=>a.localeCompare(b,undefined,{numeric:true,sensitivity:'base'});
export function videoCatalog(rows,query=''){
  const images=new Map();
  for(const row of rows.filter(row=>/\.(jpe?g|png|webp)$/i.test(row.name))){
    if(!images.has(row.folder))images.set(row.folder,new Map());
    const names=images.get(row.folder),key=stem(row.name).toLowerCase();
    if(!names.has(key))names.set(key,row.id);
  }
  function cover(folder,base){
    for(const wanted of [base,'poster','cover','folder'].filter(Boolean)){
      const image=images.get(folder)?.get(wanted.toLowerCase());
      if(image)return image;
    }
  }
  const movies=[],shows=new Map();
  for(const file of searchMedia(rows,query,'video')){
    const parts=file.folder?file.folder.split('/'):[];
    const seasonIndex=parts.findIndex(part=>/^(?:(?:season|temporada|s)[ ._-]*\d{1,3}|specials|especiales)$/i.test(part));
    const episode=/^(.*?)[ ._-]*s(\d{1,3})[ ._-]*e(\d{1,4})(?:\D|$)/i.exec(stem(file.name));
    if(seasonIndex<0&&!episode){movies.push({...file,title:title(file.name),poster:cover(file.folder,stem(file.name))});continue;}
    const folder=seasonIndex>=0?parts.slice(0,seasonIndex).join('/'):file.folder;
    const showTitle=(folder?folder.split('/').at(-1):episode?.[1].replace(/[._]+/g,' ').trim())||null;
    const key=`show:${folder||episode?.[1]||''}`;
    if(!shows.has(key))shows.set(key,{key,title:showTitle,folder,poster:cover(folder),seasons:new Map()});
    const show=shows.get(key),component=parts[seasonIndex];
    const number=seasonIndex>=0?Number(/\d+/.exec(component)?.[0]||0):Number(episode[2]);
    if(!show.seasons.has(number))show.seasons.set(number,{number,episodes:[]});
    const row={...file,title:title(file.name),number:episode?Number(episode[3]):null,
      poster:cover(file.folder,stem(file.name))||show.poster};
    show.seasons.get(number).episodes.push(row);
  }
  return {movies:movies.sort((a,b)=>order(a.title,b.title)||order(a.id,b.id)),shows:[...shows.values()].map(show=>{
    const seasons=[...show.seasons.values()].sort((a,b)=>(a.number===0)-(b.number===0)||a.number-b.number);
    for(const season of seasons)season.episodes.sort((a,b)=>(a.number??Number.MAX_SAFE_INTEGER)-(b.number??Number.MAX_SAFE_INTEGER)||order(a.name,b.name)||order(a.id,b.id));
    return {...show,poster:show.poster||seasons.flatMap(season=>season.episodes).find(episode=>episode.poster)?.poster,seasons};
  }).sort((a,b)=>order(a.title||'',b.title||'')||order(a.key,b.key))};
}
