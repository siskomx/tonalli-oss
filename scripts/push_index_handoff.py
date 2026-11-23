"""Push a frozen v2 index directly from original files; never copy a host bundle."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import subprocess
import time
import uuid

import index_handoff as freeze
from adb_receipts import cat_pending

PACKAGE='org.tonalli.spike'
RESERVE_BYTES=32*1024*1024


def file_timeout(size):
    freeze.checked_sum([size])
    # One MiB/s plus a minute, capped at a week; explicit slow transfers fail visibly.
    return min(7*24*3600,60+math.ceil(size/(1024*1024)))


def prepare_and_push(receipt_path,trusted_manifest,serial,adb='adb',runner=subprocess.run,
                     timeout=60,sleep=time.sleep,cancelled=lambda:False,evidence=None):
    if not isinstance(serial,str) or not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.:\-]{0,127}',serial) or not math.isfinite(timeout) or timeout<=0:
        raise ValueError('Explicit serial and positive timeout required')
    if cancelled(): raise InterruptedError('Transfer cancelled')
    receipt=freeze.validate_export(receipt_path,trusted_manifest)
    raw,descriptor=freeze.read_json(Path(trusted_manifest))
    freeze.validate_descriptor(raw)
    manifest_hash=hashlib.sha256(raw).hexdigest()
    if manifest_hash!=receipt['manifest_sha256']: raise ValueError('Trusted descriptor changed during validation')
    total=freeze.checked_sum(record['bytes'] for record in descriptor['files'])
    incoming_bytes=freeze.checked_sum([total,len(raw)])
    required=freeze.checked_sum([incoming_bytes,total,RESERVE_BYTES])
    request=str(uuid.uuid4())
    incoming='probe-index-'+manifest_hash[:12]
    evidence=Path(evidence) if evidence else Path(receipt_path).parent/('transfer-'+request)
    evidence.mkdir()
    sequence=0

    def active():
        if cancelled(): raise InterruptedError('Transfer cancelled; incoming preserved')

    def call(*arguments,seconds=30):
        nonlocal sequence
        active();sequence+=1
        command=[adb,'-s',serial,*arguments]
        freeze.write_new(evidence/f'{sequence:04}.command.json',freeze.json_bytes(dict(command=command,timeout_seconds=seconds)))
        try:
            result=runner(command,capture_output=True,timeout=seconds,
                          creationflags=subprocess.CREATE_NO_WINDOW if os.name=='nt' else 0)
            stdout,stderr=result.stdout,result.stderr
        except subprocess.TimeoutExpired as error:
            stdout,stderr=error.stdout or b'',error.stderr or b''
            freeze.write_new(evidence/f'{sequence:04}.stdout.bin',stdout)
            freeze.write_new(evidence/f'{sequence:04}.stderr.bin',stderr)
            raise
        freeze.write_new(evidence/f'{sequence:04}.stdout.bin',stdout)
        freeze.write_new(evidence/f'{sequence:04}.stderr.bin',stderr)
        freeze.write_new(evidence/f'{sequence:04}.exit.json',freeze.json_bytes(dict(returncode=result.returncode)))
        active()
        return result

    try:
        state=call('get-state')
        if state.returncode or state.stdout.strip()!=b'device': raise ValueError('Selected adb device is not ready')
        started=call('shell','am','start','-W','-n',PACKAGE+'/.MainActivity','-a',PACKAGE+'.PREPARE_PROBE_INPUTS',
                     '--es','request_id',request,'--es','dataset_id',descriptor['dataset_id'],'--es','manifest_sha256',manifest_hash)
        if started.returncode or not re.search(rb'Status:\s*ok\b',started.stdout) or b'Error:' in started.stdout:
            raise ValueError('App preparation action failed')
        deadline=time.monotonic()+timeout
        while True:
            reply=call('exec-out','run-as',PACKAGE,'cat',f'files/probe-inputs-ready/{request}.json')
            if reply.returncode==0:
                if len(reply.stdout)>65536: raise ValueError('Oversized app preparation receipt')
                if not cat_pending(reply.stdout,f'files/probe-inputs-ready/{request}.json'):
                    ready=json.loads(reply.stdout)
                    break
            if time.monotonic()>=deadline: raise TimeoutError('App preparation receipt missing; no payload pushed')
            sleep(.25)
        if not isinstance(ready,dict) or type(ready.get('version')) is not int or ready['version']!=1 or ready.get('request_id')!=request or ready.get('dataset_id')!=descriptor['dataset_id'] or ready.get('manifest_sha256')!=manifest_hash:
            raise ValueError('App preparation receipt identity differs')
        if ready.get('status')!='ready': raise ValueError('App refused transfer preparation; existing input preserved')
        if ready.get('generation')!=descriptor['generation'] or ready.get('incoming_name')!=incoming:
            raise ValueError('App prepared another destination')
        for key,expected in [('incoming_bytes',incoming_bytes),('internal_stage_bytes',total),('reserve_bytes',RESERVE_BYTES),('required_bytes',required)]:
            if type(ready.get(key)) is not int or ready[key]!=expected: raise ValueError('App preparation space accounting differs')
        available=ready.get('available_bytes')
        if type(available) is not int or not required<=available<=freeze.MAX_LONG: raise ValueError('Insufficient device space for incoming plus internal stage')
        root=Path(receipt['index_root'])
        for record in descriptor['files']:
            active()
            path=root/record['path'];freeze.ordinary(path)
            if freeze.identity(path)!=receipt['files'][record['path']]: raise ValueError('Frozen payload identity changed')
            target=f'/sdcard/Android/data/{PACKAGE}/files/{incoming}/{record["path"]}'
            result=call('push',str(path.resolve()),target,seconds=file_timeout(record['bytes']))
            if result.returncode: raise RuntimeError('Payload push failed; partial incoming retained')
            if freeze.identity(path)!=receipt['files'][record['path']]: raise ValueError('Payload changed during transfer')
        # Recheck provenance/identities without rescanning payloads before the final marker.
        freeze.validate_export(receipt_path,trusted_manifest)
        marker=call('push',str(Path(trusted_manifest).resolve()),f'/sdcard/Android/data/{PACKAGE}/files/{incoming}/snapshot.json',seconds=file_timeout(len(raw)))
        if marker.returncode: raise RuntimeError('Descriptor push failed; no installation claimed')
        result=dict(version=1,status='transferred',dataset_id=descriptor['dataset_id'],manifest_sha256=manifest_hash,
                    request_id=request,incoming_name=incoming,pushed_files=len(descriptor['files'])+1,
                    evidence=str(evidence),note='Delivery only; app must verify all checksums/source binding/query parity before atomic install')
        freeze.write_new(evidence/'result.json',freeze.json_bytes(result))
        return result
    except BaseException as error:
        freeze.write_new(evidence/'transfer-error.json',freeze.json_bytes(dict(error_type=type(error).__name__,error=str(error),request_id=request)))
        raise


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--export-receipt',type=Path,required=True)
    parser.add_argument('--trusted-manifest',type=Path,required=True)
    parser.add_argument('--serial',required=True)
    parser.add_argument('--adb',default='adb')
    parser.add_argument('--evidence',type=Path,required=True,help='new directory; failures/raw binary output are retained')
    args=parser.parse_args()
    print(json.dumps(prepare_and_push(args.export_receipt,args.trusted_manifest,args.serial,args.adb,evidence=args.evidence)))


if __name__=='__main__':main()
