// NOMAD/Uppy supplies relativePath for directory picks and drops, sometimes with a leading slash.
export function importFolder(base,relativePath,name) {
  if(!relativePath)return base;
  const parts=relativePath.replace(/^\//,'').split('/');
  if(parts.pop()!==name||parts.some(part=>!part||part==='.'||part==='..'||part.includes('\\')))throw new Error('invalid_file');
  return [base,...parts].filter(Boolean).join('/');
}
