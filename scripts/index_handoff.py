"""Freeze bounded index metadata in place; never copy payloads or rehash the source ZIM."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re

from phone_benchmark import digest, identity

MAX_JSON = 1024 * 1024
MAX_FILES = 4096
MAX_LONG = 2**63-1
LOCKS = {'.tantivy-meta.lock','.tantivy-writer.lock'}
QUERIES = ('solar system','photosynthesis','Mexican history')


def ordinary(path, directory=False):
    if path.is_symlink() or (hasattr(path,'is_junction') and path.is_junction()) or not (path.is_dir() if directory else path.is_file()):
        raise ValueError('Handoff inputs must be ordinary files/directories')


def read_json(path, maximum=MAX_JSON):
    ordinary(path)
    with path.open('rb') as source:
        raw=source.read(maximum+1)
    if len(raw)>maximum: raise ValueError('Oversized handoff JSON')
    return raw,json.loads(raw)


def checked_sum(values):
    total=0
    for value in values:
        if type(value) is not int or value<0 or total>MAX_LONG-value:
            raise ValueError('Handoff byte count overflows signed Long')
        total+=value
    return total


def strict_json(raw):
    def unique(pairs):
        result={}
        for key,value in pairs:
            if key in result: raise ValueError('Duplicate handoff JSON field')
            result[key]=value
        return result
    def invalid_constant(_value):
        raise ValueError('Nonfinite handoff JSON constant')
    return json.loads(raw,object_pairs_hook=unique,parse_constant=invalid_constant)


def safe_name(name):
    return isinstance(name,str) and bool(re.fullmatch(r'[A-Za-z0-9._-]{1,160}',name)) and name not in {'.','..',*LOCKS} and not name.endswith('.')


def bounded_text(text,limit):
    return isinstance(text,str) and bool(text) and len(text.encode('utf-8'))<=limit and not any(ord(c)<32 or ord(c)==127 for c in text)


def validate_descriptor(raw):
    if len(raw)>MAX_JSON: raise ValueError('Oversized handoff descriptor')
    value=strict_json(raw)
    if not isinstance(value,dict) or set(value)!={'version','dataset_id','generation','source','files','queries'} or type(value['version']) is not int or value['version']!=2 or value['dataset_id'] not in ('small','full','medicine'):
        raise ValueError('Invalid v2 descriptor')
    generation=value['generation']
    if not isinstance(generation,str) or not re.fullmatch(r'generation-[A-Za-z0-9_-]{1,64}',generation):
        raise ValueError('Invalid generation')
    source=value['source']
    if not isinstance(source,dict) or set(source)!={'uuid','bytes','md5','expected_sha256'} or type(source['bytes']) is not int or not 0<source['bytes']<=MAX_LONG:
        raise ValueError('Invalid source identity')
    for key,length in [('uuid',32),('md5',32),('expected_sha256',64)]:
        if not isinstance(source[key],str) or not re.fullmatch('[a-f0-9]{'+str(length)+'}',source[key]): raise ValueError('Invalid source digest')
    records=value['files']
    if not isinstance(records,list) or not 3<=len(records)<=MAX_FILES: raise ValueError('Invalid file count')
    names=set()
    for record in records:
        if not isinstance(record,dict) or set(record)!={'path','bytes','sha256'}: raise ValueError('Invalid file record')
        name=record['path']
        if not isinstance(name,str): raise ValueError('Invalid file path')
        parts=name.split('/')
        if name!='CURRENT' and not (len(parts)==2 and parts[0]==generation and safe_name(parts[1])): raise ValueError('File escapes generation')
        if name.casefold() in names: raise ValueError('Duplicate file record')
        names.add(name.casefold())
        if not isinstance(record['sha256'],str) or not re.fullmatch('[a-f0-9]{64}',record['sha256']): raise ValueError('Invalid file checksum')
        checked_sum([record['bytes']])
        if name=='CURRENT' and not 0<record['bytes']<=256: raise ValueError('Invalid CURRENT size')
    if not {'current',generation.casefold()+'/meta.json',generation.casefold()+'/tonalli-meta.json'}.issubset(names): raise ValueError('Missing index metadata')
    total=checked_sum(record['bytes'] for record in records)
    checked_sum([total,total,len(raw),32*1024*1024])
    queries=value['queries']
    if not isinstance(queries,list) or [q.get('query') if isinstance(q,dict) else None for q in queries]!=list(QUERIES): raise ValueError('Unexpected query baseline')
    for query in queries:
        if set(query)!={'query','hits'} or not isinstance(query['hits'],list) or len(query['hits'])>5: raise ValueError('Invalid query result')
        for hit in query['hits']:
            if not isinstance(hit,dict) or set(hit)!={'title','url'} or not bounded_text(hit['title'],8192) or not bounded_text(hit['url'],4096) or '://' in hit['url'] or hit['url'].startswith('/'):
                raise ValueError('Invalid query hit')
    return value


def hash_payload(path,cancelled):
    hasher=hashlib.sha256()
    with path.open('rb') as source:
        while True:
            if cancelled(): raise InterruptedError('Index export cancelled')
            block=source.read(1024*1024)
            if not block: break
            hasher.update(block)
    return hasher.hexdigest()


def json_bytes(value):
    return (json.dumps(value,indent=2,ensure_ascii=True,allow_nan=False)+'\n').encode()


def write_new(path,raw):
    with path.open('xb') as output:
        output.write(raw);output.flush();os.fsync(output.fileno())


def source_binding(verified_path,build):
    raw,verified=read_json(verified_path)
    source=Path(verified['identity']['path'])
    ordinary(source)
    if identity(source)!=verified['identity'] or verified['actualSha256']!=verified['source']['sha256'] or verified['source']['bytes']!=source.stat().st_size or Path(build['zim']).resolve()!=source.resolve():
        raise ValueError('Verified source receipt/identity differs')
    return source,verified,hashlib.sha256(raw).hexdigest()


def export(index_root,build_report,verified_source,output,dataset_id,cancelled=lambda:False):
    index_root,build_report,verified_source,output=map(Path,(index_root,build_report,verified_source,output))
    if output.exists(): raise FileExistsError(output)
    ordinary(index_root,directory=True)
    build_raw,build=read_json(build_report)
    source,verified,verified_sha=source_binding(verified_source,build)
    if Path(build['output']).resolve()!=index_root.resolve(): raise ValueError('Build report names another index')
    current=index_root/'CURRENT'
    ordinary(current)
    current_raw=current.read_bytes() if current.stat().st_size<=256 else b''
    generation=current_raw.decode('utf-8').strip()
    if generation!=build['generation'] or not re.fullmatch(r'generation-[A-Za-z0-9_-]{1,64}',generation): raise ValueError('Build generation differs')
    folder=index_root/generation
    ordinary(folder,directory=True)
    paths=[current]
    for path in sorted(folder.iterdir()):
        ordinary(path)
        if path.name in LOCKS: continue
        if not safe_name(path.name): raise ValueError('Invalid index filename')
        paths.append(path)
    if not 3<=len(paths)<=MAX_FILES: raise ValueError('Invalid index file count')
    _,meta=read_json(folder/'tonalli-meta.json')
    if meta!=build['index_meta'] or meta.get('format_version')!=2: raise ValueError('Build metadata differs')
    def hex16(values):
        if not isinstance(values,list) or len(values)!=16 or any(type(x) is not int or not 0<=x<=255 for x in values): raise ValueError('Invalid core identity')
        return bytes(values).hex()
    before={path.relative_to(index_root).as_posix():identity(path) for path in paths}
    provenance={str(path.resolve()):identity(path) for path in [build_report,verified_source]}
    output.mkdir()
    try:
        records=[]
        for path in paths:
            checksum=hash_payload(path,cancelled)
            name=path.relative_to(index_root).as_posix()
            if identity(path)!=before[name]: raise ValueError('Index file changed during freeze')
            records.append(dict(path=name,bytes=int(before[name]['bytes']),sha256=checksum))
        if current.read_bytes()!=current_raw or identity(source)!=verified['identity'] or any(identity(index_root/name)!=value for name,value in before.items()) or any(identity(Path(name))!=value for name,value in provenance.items()):
            raise ValueError('Source/index identity changed during freeze')
        if {p.name for p in folder.iterdir() if p.name not in LOCKS}!={p.name for p in paths[1:]}:
            raise ValueError('Generation file set changed during freeze')
        descriptor=dict(version=2,dataset_id=dataset_id,generation=generation,
            source=dict(uuid=hex16(meta['zim_uuid']),md5=hex16(meta['zim_md5']),bytes=source.stat().st_size,expected_sha256=verified['actualSha256']),
            files=records,queries=build['queries'])
        raw=json_bytes(descriptor);validate_descriptor(raw)
        receipt=dict(version=1,status='frozen',manifest_sha256=hashlib.sha256(raw).hexdigest(),index_root=str(index_root.resolve()),
            source_receipt=str(verified_source.resolve()),source_receipt_sha256=verified_sha,source_identity=verified['identity'],
            build_report=str(build_report.resolve()),build_report_sha256=hashlib.sha256(build_raw).hexdigest(),
            files=before,provenance=provenance,verification='index files SHA256 once in place; source prior verified SHA256 plus unchanged strong identity')
        write_new(output/'export-receipt.json',json_bytes(receipt))
        write_new(output/'snapshot.json',raw)
        return descriptor
    except BaseException as error:
        write_new(output/'export-error.json',json_bytes(dict(error_type=type(error).__name__,error=str(error))))
        raise


def validate_export(receipt_path,trusted_manifest):
    raw,receipt=read_json(Path(receipt_path))
    trusted_raw,_=read_json(Path(trusted_manifest))
    descriptor=validate_descriptor(trusted_raw)
    frozen=Path(receipt_path).parent/'snapshot.json'
    frozen_raw,_=read_json(frozen)
    if frozen_raw!=trusted_raw or receipt.get('version')!=1 or receipt.get('status')!='frozen' or receipt.get('manifest_sha256')!=hashlib.sha256(trusted_raw).hexdigest():
        raise ValueError('Trusted descriptor differs from frozen export')
    root=Path(receipt['index_root']);ordinary(root,directory=True)
    ordinary(root/descriptor['generation'],directory=True)
    if set(receipt['files'])!={r['path'] for r in descriptor['files']}: raise ValueError('Receipt file set differs')
    for record in descriptor['files']:
        path=root/record['path'];ordinary(path)
        if identity(path)!=receipt['files'][record['path']] or path.stat().st_size!=record['bytes']: raise ValueError('Frozen index identity changed')
    if (root/'CURRENT').read_text().strip()!=descriptor['generation']: raise ValueError('Frozen CURRENT changed')
    if {p.name for p in (root/descriptor['generation']).iterdir() if p.name not in LOCKS}!={Path(record['path']).name for record in descriptor['files'] if record['path']!='CURRENT'}:
        raise ValueError('Frozen generation file set changed')
    for name,expected in receipt['provenance'].items():
        if identity(Path(name))!=expected: raise ValueError('Frozen provenance identity changed')
    build_raw,build=read_json(Path(receipt['build_report']))
    _,verified,verified_sha=source_binding(Path(receipt['source_receipt']),build)
    if hashlib.sha256(build_raw).hexdigest()!=receipt['build_report_sha256'] or verified_sha!=receipt['source_receipt_sha256'] or verified['identity']!=receipt['source_identity']:
        raise ValueError('Frozen provenance changed')
    if descriptor['source']['expected_sha256']!=verified['actualSha256'] or descriptor['source']['bytes']!=verified['source']['bytes']:
        raise ValueError('Descriptor source differs from verified provenance')
    return receipt


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--index',type=Path,required=True)
    parser.add_argument('--build-report',type=Path,required=True)
    parser.add_argument('--verified-source',type=Path,required=True)
    parser.add_argument('--out',type=Path,required=True)
    parser.add_argument('--dataset-id',choices=('small','full','medicine'),required=True)
    args=parser.parse_args()
    value=export(args.index,args.build_report,args.verified_source,args.out,args.dataset_id)
    print(json.dumps(dict(status='frozen',output=str(args.out),files=len(value['files']))))


if __name__=='__main__':main()
