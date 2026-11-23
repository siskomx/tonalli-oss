from pathlib import Path
import hashlib,json,struct,zipfile
out=Path('web-stub/tests/fixtures/workshop');out.mkdir(exist_ok=False)
vertices=[(-2,-1,0),(2,-1,0),(2,1,0),(-2,1,0),(-2,-1,1),(2,-1,1),(2,1,1),(-2,1,1)]
quads=[(0,3,2,1),(4,5,6,7),(0,1,5,4),(1,2,6,5),(2,3,7,6),(3,0,4,7)]
faces=[tri for a,b,c,d in quads for tri in [(a,b,c),(a,c,d)]]
ascii='solid original\n'+''.join('facet normal 0 0 0\nouter loop\n'+''.join('vertex '+' '.join(map(str,vertices[i]))+'\n' for i in face)+'endloop\nendfacet\n' for face in faces)+'endsolid original\n'
(out/'Block-ascii.stl').write_text(ascii,encoding='utf-8',newline='\n')
binary=b'Tonalli original bounded model fixture'.ljust(80,b'\0')+struct.pack('<I',len(faces))
for face in faces:binary+=struct.pack('<12fH',0,0,0,*(v for i in face for v in vertices[i]),0)
(out/'Block-binary.STL').write_bytes(binary)
obj='# Original Tonalli fixture, GPL-3.0-or-later\nmtllib https://example.invalid/never-fetch.mtl\n'+''.join('v '+' '.join(map(str,p))+'\n' for p in vertices)+''.join('f '+' '.join(str(i+1) for i in face)+'\n' for face in quads)
(out/'Block.obj').write_text(obj,encoding='utf-8',newline='\n')
xml='<model xmlns="http://schemas.microsoft.com/3dmanufacturing/core/2015/02" unit="millimeter"><metadata name="Title">Original Tonalli component fixture</metadata><resources><object id="1" type="model"><mesh><vertices>'+''.join(f'<vertex x="{x}" y="{y}" z="{z}"/>' for x,y,z in vertices)+'</vertices><triangles>'+''.join(f'<triangle v1="{a}" v2="{b}" v3="{c}"/>' for a,b,c in faces)+'</triangles></mesh></object><object id="2"><components><component objectid="1"/><component objectid="1" transform="1 0 0 0 1 0 0 0 1 5 0 0"/></components></object></resources><build><item objectid="2" transform="1 0 0 0 1 0 0 0 1 0 3 0"/></build></model>'
with zipfile.ZipFile(out/'Components.3mf','w',zipfile.ZIP_DEFLATED) as archive:
 for name,text in {'[Content_Types].xml':'<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>','_rels/.rels':'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="r0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel" Target="/3D/main.model"/></Relationships>','3D/main.model':xml}.items():
  info=zipfile.ZipInfo(name,date_time=(2026,11,13,0,0,0));info.compress_type=zipfile.ZIP_DEFLATED;archive.writestr(info,text.encode())
(out/'Broken.stl').write_bytes(b'Invalid model'.ljust(80,b'\0')+struct.pack('<I',0xffffffff))
with zipfile.ZipFile(out/'Components.3mf') as archive:
 assert archive.testzip() is None
 assert archive.read('3D/main.model').decode()==xml
assert len(faces)==12 and len(binary)==84+50*12
records=[{'name':p.name,'bytes':p.stat().st_size,'sha256':hashlib.sha256(p.read_bytes()).hexdigest()} for p in out.iterdir()]
(out/'manifest.json').write_text(json.dumps({'license':'GPL-3.0-or-later','version':'v1','original':True,'files':records},indent=2)+'\n',encoding='utf-8')
print(json.dumps(records))
