import {it,expect} from 'vitest';
import {mergeRoom} from '../../src/features/room/client.js';
const message=id=>({id,text:`literal <script>${id}</script>`,sender:{id:'peer',label:'Peer',is_host:false},created_ms:1000});
const joined=(epoch,messages,latest)=>({active:true,joined:true,epoch,self_id:'peer',messages,members:[],latest});
it('bounds and deduplicates the visible room tail without reverting a newer response',()=>{
 const old=joined('one',Array.from({length:100},(_,i)=>message(i+1)),100);
 const value=mergeRoom(old,joined('one',[message(100),message(101)],101));expect(value.messages.length).toBe(100);expect(value.messages[0].id).toBe(2);expect(value.messages.at(-1).text).toBe('literal <script>101</script>');
 expect(mergeRoom(value,joined('one',[],100))).toBe(value);
});
it('clears cached text on room change, leave or sharing stop',()=>{
 const old=joined('one',[message(1)],1);
 expect(mergeRoom(old,joined('two',[message(2)],2)).messages).toEqual([message(2)]);
 for(const incoming of [{...old,active:false},{...old,joined:false}]){const value=mergeRoom(old,incoming);expect(value.messages).toEqual([]);expect(value.members).toEqual([]);expect(value.latest).toBe(0);}
 expect(()=>mergeRoom(null,joined('one',Array.from({length:101},(_,i)=>message(i)),100))).toThrow();
});
