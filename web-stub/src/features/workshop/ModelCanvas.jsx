import React,{useEffect,useRef,useState} from 'react';
import * as THREE from 'three';
import {OrbitControls} from 'three/addons/controls/OrbitControls.js';
import {useApp} from '../../app/context.js';

export default function ModelCanvas({model,route}){
  const{t}=useApp(),host=useRef(null),actions=useRef(null),callbacks=useRef(route);callbacks.current=route;
  const[failed,setFailed]=useState(false),[ready,setReady]=useState(false),[attempt,setAttempt]=useState(0);
  useEffect(()=>{
    let renderer,geometry,material,controls,observer,closed=false,canvas;
    setReady(false);setFailed(false);
    function unavailable(){if(closed)return;setFailed(true);setReady(false);void callbacks.current.markReady({feature:'workshop',unsupported:true});}
    function lost(event){event.preventDefault();unavailable();}
    try{
      const values=model.positions;
      if(!Array.isArray(values)||!values.length||values.length%9||values.length>900000||values.some(value=>!Number.isFinite(value)||Math.abs(value)>1e12))throw Error('Invalid geometry');
      geometry=new THREE.BufferGeometry();geometry.setAttribute('position',new THREE.Float32BufferAttribute(values,3));geometry.computeBoundingBox();
      const box=geometry.boundingBox,size=box.getSize(new THREE.Vector3()),extent=Math.max(size.x,size.y,size.z),center=box.getCenter(new THREE.Vector3());
      if(!Number.isFinite(extent)||extent<1e-9)throw Error('Empty geometry');
      geometry.translate(-center.x,-center.y,-center.z);geometry.scale(2/extent,2/extent,2/extent);geometry.computeVertexNormals();
      renderer=new THREE.WebGLRenderer({antialias:true});renderer.setPixelRatio(Math.min(devicePixelRatio,2));renderer.setClearColor(0xf4f1e8);
      canvas=renderer.domElement;canvas.setAttribute('aria-label',t('workshop.canvas'));canvas.setAttribute('role','img');canvas.tabIndex=0;canvas.style.maxWidth='100%';canvas.addEventListener('webglcontextlost',lost);host.current.appendChild(canvas);
      const scene=new THREE.Scene(),camera=new THREE.PerspectiveCamera(40,1,.01,100);
      camera.position.set(3,2,4);const initial=camera.position.clone();camera.up.set(0,0,1);
      material=new THREE.MeshStandardMaterial({color:0xc98b3d,roughness:.8,metalness:0,side:THREE.DoubleSide,flatShading:true});
      const mesh=new THREE.Mesh(geometry,material);scene.add(mesh);scene.add(new THREE.HemisphereLight(0xffffff,0x615b52,2));const light=new THREE.DirectionalLight(0xffffff,3);light.position.set(4,3,5);scene.add(light);
      controls=new OrbitControls(camera,canvas);controls.enablePan=false;controls.minDistance=1.5;controls.maxDistance=15;
      function render(){if(!closed)renderer.render(scene,camera);}
      controls.addEventListener('change',render);
      actions.current={rotate:angle=>{mesh.rotation.z+=angle;render();},zoom:factor=>{const distance=Math.max(controls.minDistance,Math.min(controls.maxDistance,camera.position.length()*factor));camera.position.setLength(distance);controls.update();render();},reset:()=>{mesh.rotation.set(0,0,0);camera.position.copy(initial);controls.target.set(0,0,0);controls.update();render();}};
      canvas.onkeydown=event=>{if(['ArrowLeft','ArrowRight','+','-','Home'].includes(event.key)){event.preventDefault();if(event.key==='Home')actions.current.reset();else if(event.key==='+'||event.key==='-')actions.current.zoom(event.key==='+'?.8:1.25);else actions.current.rotate(event.key==='ArrowLeft'?.25:-.25);}};
      observer=new ResizeObserver(()=>{const target=host.current;if(closed||!target)return;const width=target.clientWidth,height=target.clientHeight;if(width&&height){renderer.setSize(width,height);camera.aspect=width/height;camera.updateProjectionMatrix();render();}});observer.observe(host.current);
      controls.update();render();setReady(true);void callbacks.current.markReady({feature:'workshop',triangles:values.length/9});
    }catch{unavailable();}
    return()=>{closed=true;actions.current=null;observer?.disconnect();controls?.dispose();geometry?.dispose();material?.dispose();canvas?.removeEventListener('webglcontextlost',lost);renderer?.dispose();renderer?.forceContextLoss();canvas?.remove();};
  },[model,t,attempt]);
  return <section className="space-y-3" data-testid="model-preview">
    <p>{t('workshop.controls')}</p><div className="flex flex-wrap gap-2">
      <button className="secondary-button" disabled={!ready} onClick={()=>actions.current?.rotate(.25)}>{t('workshop.left')}</button><button className="secondary-button" disabled={!ready} onClick={()=>actions.current?.rotate(-.25)}>{t('workshop.right')}</button>
      <button className="secondary-button" disabled={!ready} onClick={()=>actions.current?.zoom(.8)}>{t('workshop.zoomIn')}</button><button className="secondary-button" disabled={!ready} onClick={()=>actions.current?.zoom(1.25)}>{t('workshop.zoomOut')}</button><button className="secondary-button" disabled={!ready} onClick={()=>actions.current?.reset()}>{t('workshop.reset')}</button>
    </div><div ref={host} className="w-full h-[60dvh] min-h-80 max-h-[700px] overflow-hidden rounded border touch-none" data-testid="model-canvas"/>
    {failed&&<><p role="alert">{t('workshop.graphicsFailed')}</p><button className="secondary-button" onClick={()=>setAttempt(value=>value+1)}>{t('files.refresh')}</button></>}
  </section>;
}
