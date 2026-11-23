// Cooklang core markers; keep notes/metadata and unknown syntax visible as text.
// No evaluation, file references, unit arithmetic or network recipe resolution.
export function parseRecipe(source){
  if(typeof source!=='string'||new TextEncoder().encode(source).length>65536)throw Error('recipe_limit');
  const recipe={ingredients:[],steps:[],notes:[]};let paragraph=[],front=false;
  function flush(){if(paragraph.length){recipe.steps.push(paragraph.join(' ').replace(/\\ /g,'\n'));paragraph=[];}}
  const clean=source.replace(/\r\n?/g,'\n').replace(/\[-[\s\S]*?-\]/g,'');
  function markers(line){
    return line.replace(/([@#~])(\??)(?:([^@#~{}\n]{0,256})\{([^{}\n]{0,256})\}|([\p{L}\p{N}_]+(?:[-'][\p{L}\p{N}_]+)*))/gu,(whole,sigil,optional,multi,amount,single)=>{
      const name=(multi??single).trim(),quantity=(amount||'').replace('%',' ').trim();
      if(!name&&sigil!=='~')return whole;
      const label=`${name}${optional?' (optional)':''}`;
      if(sigil==='@')recipe.ingredients.push(`${label}${quantity?` — ${quantity}`:''}`);
      return sigil==='~'?[name,quantity].filter(Boolean).join(' '):`${label}${quantity?` (${quantity})`:''}`;
    });
  }
  for(const [index,raw] of clean.split('\n').entries()){
    if(raw.trim()==='---'&&(index===0||front)){flush();front=!front;continue;}
    if(front){recipe.notes.push(raw);continue;}
    const line=raw.replace(/--.*$/,'').trim();
    if(!line){flush();continue;}
    if(line.startsWith('>')||line.startsWith('=')){flush();recipe.notes.push(line.replace(/^(?:>+|=+)\s*/,'').replace(/=+$/,'').trim());continue;}
    paragraph.push(markers(line));
  }
  flush();
  if(recipe.ingredients.length>128||recipe.steps.length>256||!recipe.steps.length)throw Error('recipe_limit');
  return recipe;
}
export function recipePosition(hash,ingredients,steps){return {kind:'epub',location:JSON.stringify({reader:'recipe',hash,ingredients,steps})};}
export function recipeProgress(position,hash,recipe){
  try{
    const value=position?.kind==='epub'&&JSON.parse(position.location);
    const valid=(list,length)=>Array.isArray(list)&&list.length<=length&&new Set(list).size===list.length&&list.every(index=>Number.isInteger(index)&&index>=0&&index<length);
    if(value.reader==='recipe'&&value.hash===hash&&valid(value.ingredients,recipe.ingredients.length)&&valid(value.steps,recipe.steps.length))return {ingredients:value.ingredients,steps:value.steps};
  }catch{}
  return {ingredients:[],steps:[]};
}
