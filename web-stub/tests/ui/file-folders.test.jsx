import {expect,test} from 'vitest';
import {importFolder} from '../../src/features/files/folders.js';
test('NOMAD directory/drop metadata keeps nested folders under the chosen label',()=>{
  expect(importFolder('Trips','Movies/Season 1/one.mp4','one.mp4')).toBe('Trips/Movies/Season 1');
  expect(importFolder('','/Movies/Season 1/one.mp4','one.mp4')).toBe('Movies/Season 1');
  expect(importFolder('Trips','','one.mp4')).toBe('Trips');
  expect(()=>importFolder('Trips','../private/file.txt','file.txt')).toThrow();
  expect(()=>importFolder('','folder/different.txt','file.txt')).toThrow();
});
