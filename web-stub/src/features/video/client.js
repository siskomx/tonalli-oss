export async function videoRequest(csrf,path,body,signal){
  const response=await fetch(`/api/video/${path}`,{method:'POST',signal,credentials:'same-origin',cache:'no-store',keepalive:path==='progress',
    headers:{'X-Tonalli-Csrf':csrf,'Content-Type':'application/json'},body:JSON.stringify(body)});
  if(!response.ok)throw Error(response.status===415?'video_unsupported':'video_unavailable');
  return response.json();
}
