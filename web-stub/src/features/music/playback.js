export function playbackStart(seconds,duration,seeking){
  if(!Number.isFinite(duration)||duration<=0)return seconds;
  return !seeking&&seconds>=duration?0:Math.min(seconds,duration);
}
