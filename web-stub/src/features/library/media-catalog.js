// Filename categories are browsing hints; individual readers must validate content.
const extensions={
  music:['mp3','m4a','aac','flac','ogg','opus','wav'],
  video:['mp4','m4v','webm','mkv','avi','mov'],
  images:['jpg','jpeg','png','gif','webp','avif','bmp'],
  books:['epub','pdf'],comics:['cbz','cbr'],
  recipes:['cook'],workshop:['stl','3mf','obj'],
};
export function mediaCategory(name){
  const extension=name.split('.').at(-1).toLowerCase();
  return Object.entries(extensions).find(([,values])=>values.includes(extension))?.[0]||'files';
}
export function mediaHref(file){
  const category=mediaCategory(file.name);
  if(category==='music')return `/music?${new URLSearchParams({track:file.id})}`;
  if(category==='video')return `/videos?${new URLSearchParams({file:file.id})}`;
  if(category==='images')return `/gallery?${new URLSearchParams({file:file.id})}`;
  if(['books','comics'].includes(category))return `/reader?${new URLSearchParams({file:file.id})}`;
  if(category==='recipes')return `/cookbook?${new URLSearchParams({file:file.id})}`;
  if(category==='workshop')return `/workshop?${new URLSearchParams({file:file.id})}`;
  return `/file?${new URLSearchParams({file:file.id})}`;
}
export function searchMedia(rows,query,category='all'){
  const term=query.trim().toLowerCase();
  return rows.filter(row=>(category==='all'||mediaCategory(row.name)===category)
    &&(!term||`${row.name}\n${row.folder}`.toLowerCase().includes(term)));
}
