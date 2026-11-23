"""Synthetic/local metadata-only handoff checks. Never invokes a device."""
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import index_handoff as freeze
import push_index_handoff as push


class HandoffTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.index = self.root/'index'
        self.generation = 'generation-synthetic'
        folder = self.index/self.generation
        folder.mkdir(parents=True)
        (self.index/'CURRENT').write_text(self.generation)
        self.meta = {'zim_uuid': list(range(16)), 'zim_md5': list(range(16,32)),
                     'format_version': 2, 'analyzer_revision': 1, 'std_unicode_version': [17,0,0]}
        (folder/'tonalli-meta.json').write_text(json.dumps(self.meta))
        (folder/'meta.json').write_text('{"segments":[]}')
        (folder/'data.idx').write_bytes(b'synthetic index payload')
        (folder/'.tantivy-meta.lock').write_bytes(b'not exported')
        source = self.root/'source.zim'
        source.write_bytes(b'synthetic source; not a real ZIM')
        sha = hashlib.sha256(source.read_bytes()).hexdigest()
        self.verified = self.root/'verified.json'
        self.verified.write_text(json.dumps({'identity':freeze.identity(source), 'actualSha256':sha,
            'source':{'bytes':source.stat().st_size,'sha256':sha}}))
        self.report = self.root/'build.json'
        self.report.write_text(json.dumps({'version':1, 'zim':str(source), 'output':str(self.index),
            'generation':self.generation, 'index_meta':self.meta,
            'queries':[{'query':q,'hits':[{'title':'Synthetic title','url':'C/Synthetic'}]} for q in freeze.QUERIES]}))
        self.output = self.root/'export'

    def exported(self):
        freeze.export(self.index,self.report,self.verified,self.output,'small')
        return self.output/'snapshot.json', self.output/'export-receipt.json'

    def test_explicit_medicine_export_validates_and_pushes_same_dataset(self):
        freeze.export(self.index,self.report,self.verified,self.output,'medicine')
        manifest=self.output/'snapshot.json';receipt=self.output/'export-receipt.json'
        descriptor=freeze.validate_descriptor(manifest.read_bytes())
        self.assertEqual(descriptor['dataset_id'],'medicine')
        with patch.object(freeze,'digest',side_effect=AssertionError('no payload/source rehash')):
            freeze.validate_export(receipt,manifest)
        state,runner=self.fake(dataset='medicine')
        result=push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=runner)
        self.assertEqual(result['status'],'transferred')
        self.assertEqual(Path([args for args,_ in state['calls'] if args[0]=='push'][-1][1]),manifest.resolve())

    def test_cli_accepts_explicit_medicine_dataset(self):
        args=['index_handoff.py','--index',str(self.index),'--build-report',str(self.report),
              '--verified-source',str(self.verified),'--out',str(self.output),'--dataset-id','medicine']
        with patch('sys.argv',args),patch('builtins.print'):
            freeze.main()
        self.assertEqual(json.loads((self.output/'snapshot.json').read_bytes())['dataset_id'],'medicine')

    def test_unknown_dataset_ids_remain_rejected(self):
        manifest,_=self.exported()
        value=json.loads(manifest.read_bytes())
        for dataset in ['unknown','medicine-es','Medicine','../medicine','']:
            with self.subTest(dataset=dataset):
                changed={**value,'dataset_id':dataset}
                with self.assertRaises(ValueError): freeze.validate_descriptor(json.dumps(changed).encode())

    def test_metadata_only_export_reuses_verified_source_and_keeps_original_files(self):
        manifest, receipt = self.exported()
        self.assertEqual(set(p.name for p in self.output.iterdir()), {'snapshot.json','export-receipt.json'})
        data=json.loads(manifest.read_bytes())
        self.assertEqual(data['version'],2)
        self.assertEqual(data['source']['uuid'],bytes(range(16)).hex())
        self.assertEqual(data['source']['md5'],bytes(range(16,32)).hex())
        self.assertEqual(len(data['files']),4)
        with patch.object(freeze,'digest',side_effect=AssertionError('no rehash at push validation')):
            trusted=freeze.validate_export(receipt,manifest)
        self.assertEqual(trusted['index_root'],str(self.index.resolve()))
        with self.assertRaises(FileExistsError):
            freeze.export(self.index,self.report,self.verified,self.output,'small')

    def test_export_accepts_moderate_payload_above_old32mib_without_copy(self):
        path=self.index/self.generation/'large.idx'
        with path.open('wb') as out:
            for _ in range(33): out.write(b'x'*(1024*1024))
        manifest,_=self.exported()
        self.assertGreater(sum(f['bytes'] for f in json.loads(manifest.read_bytes())['files']),32*1024*1024)
        self.assertLess(sum(p.stat().st_size for p in self.output.iterdir()),64*1024)

    def test_source_mismatch_and_cancel_preserve_failure_without_success_descriptor(self):
        value=json.loads(self.verified.read_bytes());value['identity']['mtimeNs']='changed'
        self.verified.write_text(json.dumps(value))
        with self.assertRaises(ValueError): self.exported()
        self.assertFalse((self.output/'snapshot.json').exists())
        self.setUp()
        with self.assertRaises(InterruptedError):
            freeze.export(self.index,self.report,self.verified,self.output,'small',cancelled=lambda:True)
        self.assertFalse((self.output/'snapshot.json').exists())

    def test_mutation_during_hash_is_rejected_and_partial_evidence_preserved(self):
        original=freeze.hash_payload
        def changing(path,cancelled):
            value=original(path,cancelled)
            if path.name=='data.idx': path.write_bytes(b'changed during freeze')
            return value
        with patch.object(freeze,'hash_payload',changing), self.assertRaises(ValueError): self.exported()
        self.assertTrue((self.output/'export-error.json').is_file())
        self.assertFalse((self.output/'snapshot.json').exists())

    def test_descriptor_bounds_and_identity_changes_fail_before_any_device_call(self):
        manifest,receipt=self.exported()
        (self.index/self.generation/'data.idx').write_bytes(b'changed')
        with self.assertRaises(ValueError):
            push.prepare_and_push(receipt,manifest,'explicit',runner=lambda *a,**k:self.fail('no adb'))
        data=json.loads(manifest.read_bytes())
        for record in [{'path':'../outside','bytes':1,'sha256':'a'*64},
                       {'path':self.generation+'/data.idx','bytes':2**63,'sha256':'a'*64}]:
            changed={**data,'files':[data['files'][0],record]}
            with self.assertRaises(ValueError): freeze.validate_descriptor(json.dumps(changed).encode())

    def test_current_mutation_added_file_and_query_url_escape_are_rejected(self):
        manifest,receipt=self.exported()
        (self.index/self.generation/'extra.idx').write_bytes(b'late writer')
        with self.assertRaises(ValueError): freeze.validate_export(receipt,manifest)
        data=json.loads(manifest.read_bytes())
        data['queries'][0]['hits'][0]['url']='https://external.invalid/'
        with self.assertRaises(ValueError): freeze.validate_descriptor(json.dumps(data).encode())
        with self.assertRaises(ValueError): freeze.validate_descriptor(b' '*(freeze.MAX_JSON+1))

    def test_duplicate_json_fields_counts_and_accounting_overflow_are_rejected(self):
        manifest,_=self.exported();raw=manifest.read_bytes()
        with self.assertRaises(ValueError): freeze.validate_descriptor(raw.replace(b'"version": 2',b'"version": 2, "version": 2',1))
        value=json.loads(raw)
        value['files'].extend([value['files'][-1]]*freeze.MAX_FILES)
        with self.assertRaises(ValueError): freeze.validate_descriptor(json.dumps(value).encode())
        value=json.loads(raw)
        value['files'][-1]['bytes']=freeze.MAX_LONG//2
        with self.assertRaises(ValueError): freeze.validate_descriptor(json.dumps(value).encode())
        for invalid in [True,1.5,-1]:
            with self.assertRaises(ValueError): freeze.checked_sum([invalid])

    def test_current_reselection_after_freeze_is_rejected(self):
        manifest,receipt=self.exported()
        (self.index/'CURRENT').write_text('generation-other')
        with self.assertRaises(ValueError): freeze.validate_export(receipt,manifest)

    def fake(self,available=10**12,failed_push=False,dataset='small'):
        state={'calls':[]}
        def runner(command,**kwargs):
            self.assertEqual(command[:3],['fixture-adb','-s','explicit'])
            args=command[3:];state['calls'].append((args,kwargs))
            if args==['get-state']: raw=b'device\n'
            elif args[:3]==['shell','am','start']:
                state['request']=args[args.index('request_id')+1]
                state['hash']=args[args.index('manifest_sha256')+1]
                self.assertEqual(args[args.index('dataset_id')+1],dataset)
                raw=b'Status: ok\n'
            elif args[:4]==['exec-out','run-as',push.PACKAGE,'cat']:
                value=json.loads((self.output/'snapshot.json').read_bytes())
                total=sum(f['bytes'] for f in value['files'])
                incoming=total+(self.output/'snapshot.json').stat().st_size
                raw=json.dumps({'version':1,'request_id':state['request'],'dataset_id':dataset,
                    'manifest_sha256':state['hash'],'status':'ready','generation':self.generation,
                    'incoming_name':'probe-index-'+state['hash'][:12], 'incoming_bytes':incoming,
                    'internal_stage_bytes':total,'reserve_bytes':push.RESERVE_BYTES,
                    'required_bytes':incoming+total+push.RESERVE_BYTES,'available_bytes':available}).encode()
            elif args[0]=='push':
                self.assertTrue(Path(args[1]).is_file())
                raw=b'1 file pushed\n'
                if failed_push: return subprocess.CompletedProcess(command,1,b'',b'raw synthetic failure')
            else:self.fail(args)
            return subprocess.CompletedProcess(command,0,raw,b'')
        return state,runner

    def test_direct_push_uses_original_files_and_marker_last_scaled_timeout(self):
        manifest,receipt=self.exported();state,runner=self.fake()
        result=push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=runner)
        calls=[(args,kw) for args,kw in state['calls'] if args[0]=='push']
        self.assertEqual(result['status'],'transferred')
        self.assertEqual(Path(calls[-1][0][1]),manifest.resolve())
        self.assertTrue(all(Path(args[1]).is_relative_to(self.index) for args,_ in calls[:-1]))
        self.assertGreater(push.file_timeout(5*1024**3),30)
        for args,kw in calls[:-1]:
            self.assertEqual(kw['timeout'],push.file_timeout(Path(args[1]).stat().st_size))
        self.assertFalse(any(any(x in args for x in ['mkdir','chmod','rm']) for args,_ in state['calls']))

    def test_space_refusal_cancel_and_failed_push_never_publish_marker(self):
        manifest,receipt=self.exported();state,runner=self.fake(available=1)
        with self.assertRaises(ValueError): push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=runner)
        self.assertFalse(any(args[0]=='push' for args,_ in state['calls']))
        state,runner=self.fake(failed_push=True)
        with self.assertRaises(RuntimeError): push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=runner)
        self.assertEqual(len([a for a,_ in state['calls'] if a[0]=='push']),1)
        with self.assertRaises(InterruptedError):
            push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=runner,cancelled=lambda:True)

    def test_exact_pending_cat_retries_but_published_bad_json_fails_fast(self):
        manifest,receipt=self.exported();state,fake=self.fake()
        pending=[b'',None]
        def race(command,**kwargs):
            value=fake(command,**kwargs)
            if command[3]=='exec-out' and pending:
                sample=pending.pop(0)
                value.stdout=sample if sample is not None else f'cat: {command[-1]}: No such file or directory'.encode()
            return value
        push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=race,sleep=lambda _:None)
        self.assertEqual(len([a for a,_ in state['calls'] if a[0]=='exec-out']),3)
        state,fake=self.fake()
        def malformed(command,**kwargs):
            value=fake(command,**kwargs)
            if command[3]=='exec-out':value.stdout=b'{published malformed'
            return value
        with self.assertRaises(ValueError):
            push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=malformed)
        self.assertFalse(any(a[0]=='push' for a,_ in state['calls']))

    def test_payload_timeout_preserves_binary_failure_and_old_evidence(self):
        manifest,receipt=self.exported();state,fake=self.fake()
        evidence=self.root/'transfer-evidence'
        def timeout(command,**kwargs):
            value=fake(command,**kwargs)
            if command[3]=='push':
                raise subprocess.TimeoutExpired(command,kwargs['timeout'],output=b'partial\x00stdout',stderr=b'raw stderr\xff')
            return value
        with self.assertRaises(subprocess.TimeoutExpired):
            push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=timeout,evidence=evidence)
        self.assertEqual((evidence/'0004.stdout.bin').read_bytes(),b'partial\x00stdout')
        self.assertEqual((evidence/'0004.stderr.bin').read_bytes(),b'raw stderr\xff')
        self.assertTrue((evidence/'transfer-error.json').is_file())
        with self.assertRaises(FileExistsError):
            push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=lambda *a,**k:self.fail('no adb'),evidence=evidence)

    def test_stale_receipt_and_cancel_during_transfer_never_push_marker(self):
        manifest,receipt=self.exported();state,fake=self.fake()
        def stale(command,**kwargs):
            value=fake(command,**kwargs)
            if command[3]=='exec-out':
                data=json.loads(value.stdout);data['dataset_id']='full';value.stdout=json.dumps(data).encode()
            return value
        with self.assertRaises(ValueError):push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=stale)
        self.assertFalse(any(a[0]=='push' for a,_ in state['calls']))
        state,fake=self.fake();cancel=[False]
        def interrupt(command,**kwargs):
            value=fake(command,**kwargs)
            if command[3]=='push':cancel[0]=True
            return value
        with self.assertRaises(InterruptedError):
            push.prepare_and_push(receipt,manifest,'explicit','fixture-adb',runner=interrupt,cancelled=lambda:cancel[0])
        pushed=[a for a,_ in state['calls'] if a[0]=='push']
        self.assertEqual(len(pushed),1)
        self.assertFalse(pushed[0][2].endswith('snapshot.json'))


if __name__=='__main__':unittest.main()
