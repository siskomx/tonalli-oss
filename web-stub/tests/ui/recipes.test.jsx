import {it,expect} from 'vitest';
import {parseRecipe,recipeProgress,recipePosition} from '../../src/features/recipes/recipe.js';

it('reads Cooklang paragraphs and ingredients without executing or hiding recipe text',()=>{
  const recipe=parseRecipe('>> title: Soup\n\n-- ignored\nMix @red lentils{200%g} and @water{1%l}.\nStir in @salt.\n\nCook in a #pot{} for ~{20%minutes}.\n\n> Keep leftovers cool.');
  expect(recipe.ingredients).toEqual(['red lentils — 200 g','water — 1 l','salt']);
  expect(recipe.steps).toEqual(['Mix red lentils (200 g) and water (1 l). Stir in salt.','Cook in a pot for 20 minutes.']);
  expect(recipe.notes).toEqual(['title: Soup','Keep leftovers cool.']);
  expect(parseRecipe('Add @?black pepper{} [- private comment -] and @salt{1/2%tsp}.').ingredients).toEqual(['black pepper (optional)','salt — 1/2 tsp']);
  expect(parseRecipe('<script>alert(1)</script>').steps).toEqual(['<script>alert(1)</script>']);
});

it('bounds recipe work and progress and refuses stale or malformed checklists',()=>{
  expect(()=>parseRecipe('é'.repeat(32769))).toThrow();
  expect(()=>parseRecipe(Array.from({length:257},()=> 'Step').join('\n\n'))).toThrow();
  const recipe=parseRecipe('Mix @rice{2%cup}.\n\nServe.');
  const position=recipePosition('hash',[0],[1]);
  expect(recipeProgress(position,'hash',recipe)).toEqual({ingredients:[0],steps:[1]});
  for(const invalid of [{kind:'start'},recipePosition('old',[0],[1]),recipePosition('hash',[99],[0]),{kind:'epub',location:'{"reader":"recipe","hash":"hash","ingredients":[0,0],"steps":[]}'}]){
    expect(recipeProgress(invalid,'hash',recipe)).toEqual({ingredients:[],steps:[]});
  }
});
