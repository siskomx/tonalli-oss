import {it,expect} from 'vitest';
import {playbackStart} from '../../src/features/music/playback.js';
it('replays completed tracks while preserving an explicit seek to the end',()=>{
  expect(playbackStart(60,60,false)).toBe(0);
  expect(playbackStart(70,60,false)).toBe(0);
  expect(playbackStart(12,60,false)).toBe(12);
  expect(playbackStart(60,60,true)).toBe(60);
  expect(playbackStart(70,60,true)).toBe(60);
  expect(playbackStart(12,NaN,false)).toBe(12);
});
