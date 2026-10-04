// Compile the actual Windows HRGN allocator and store geometry in isolation.
// GDI region queries create no windows and never touch the installed desktop.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
import { SHAPES, shapeGeometry, physicalRegions } from '../../ui/widget-geometry.mjs';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const source = fs.readFileSync(path.join(root, 'shell/packages/desktop/src/desktop_widgets.rs'), 'utf8').replace(/\r\n/g, '\n');
const derive = '#[derive(Clone, Debug, Serialize, Deserialize)]';
const regionStart = source.lastIndexOf(derive, source.indexOf('pub struct RegionPoint'));
const definitions = source.slice(regionStart, source.indexOf('#[derive(Clone)]\nstruct Host'));
const monitorStart = source.indexOf(derive);
const monitor = source.slice(monitorStart, regionStart);
const handleStart = source.indexOf('unsafe fn region_handle(');
const handle = source.slice(handleStart, source.indexOf('pub fn set_regions(', handleStart));
if (handleStart < 0) throw new Error('Missing native shape allocator');
const store = source.slice(source.indexOf('fn sizes('), source.indexOf('fn read_store('));
const clamp = source.slice(source.indexOf('fn clamp('), source.indexOf('fn overlaps('));
const fixtures = [];
const probes = { card: [[160,120,true],[0,0,false]], capsule: [[1,1,false],[160,20,true]], circle: [[1,1,false],[160,120,true]], ticket: [[1,120,false],[18,120,true],[319,120,false]], bubble: [[310,235,false],[102,232,true]], hexagon: [[1,1,false],[160,120,true]], polaroid: [[3,3,false],[160,10,true],[160,100,true]], split: [[95.6,120,false],[40,120,true],[180,120,true]] };
for (const shape of Object.keys(SHAPES)) for (const scale of [1,1.25,1.5,2]) fixtures.push({ shape, scale, regions: physicalRegions(shapeGeometry(shape,320,240), { x: 80, y: 40, scaleX: scale, scaleY: scale }), probes: probes[shape] });
const out = path.join(root, 'build/tests/desktop-widgets-geometry-native');
fs.mkdirSync(path.join(out, 'src'), { recursive: true });
fs.writeFileSync(path.join(out, 'Cargo.toml'), '[package]\nname="desktop-widgets-geometry-native-tests"\nversion="0.0.0"\nedition="2021"\n[workspace]\n[dependencies]\nserde={version="1",features=["derive"]}\nserde_json="1"\nwindows={version="0.58",features=["Win32_Foundation","Win32_Graphics_Gdi"]}\n');
fs.writeFileSync(path.join(out, 'src/lib.rs'), `#![cfg(test)]\n#![allow(dead_code)]\nuse serde::{Serialize,Deserialize}; use serde_json::{json,Value}; use windows::Win32::{Foundation::POINT,Graphics::Gdi::*};\n${monitor}\n${definitions}\n${store}\n${clamp}\n${handle}\n
#[test] fn real_hrgn_contours_at_four_dpi_scales() {
  let fixtures:Value=serde_json::from_str(r###"${JSON.stringify(fixtures)}"###).unwrap();
  unsafe { for f in fixtures.as_array().unwrap() {
    let regions:Vec<Region>=serde_json::from_value(f["regions"].clone()).unwrap();
    let all=CreateRectRgn(0,0,0,0);
    for r in &regions { assert!(valid_region(r,0)); let part=region_handle(r).unwrap(); assert_ne!(CombineRgn(all,all,part,RGN_OR),RGN_ERROR); let _=DeleteObject(part); }
    let scale=f["scale"].as_f64().unwrap();
    for p in f["probes"].as_array().unwrap() {
      let x=(80.+p[0].as_f64().unwrap()*scale).round() as i32; let y=(40.+p[1].as_f64().unwrap()*scale).round() as i32;
      assert_eq!(PtInRegion(all,x,y).as_bool(),p[2].as_bool().unwrap(),"{} scale {} point {},{}",f["shape"],scale,x,y);
    }
    let _=DeleteObject(all);
  } }
}
#[test] fn legacy_popup_rectangles_and_malformed_descriptors() {
  let r:Region=serde_json::from_value(json!({"x":10,"y":20,"w":100,"h":150})).unwrap(); assert!(valid_region(&r,0));
  unsafe { let part=region_handle(&r).unwrap(); assert!(PtInRegion(part,20,30).as_bool()); assert!(!PtInRegion(part,5,30).as_bool()); let _=DeleteObject(part); }
  for value in [json!({"x":0,"y":0,"w":100,"h":100,"shape":"polygon","points":[]}),json!({"x":0,"y":0,"w":100,"h":100,"radius":-1}),json!({"x":0,"y":0,"w":100,"h":100,"shape":"unknown"})] { let r:Region=serde_json::from_value(value).unwrap(); assert!(!valid_region(&r,0)); }
}
#[test] fn circle_store_projection_clamps_aspect_and_other_shape_minima_match() {
  let m=DesktopMonitor{device:"DISPLAY1".into(),primary:true,x:0,y:40,width:200,height:150,scale:1.25};
  let mut s=normalize(&json!({"id":1,"kind":"weather","shape":"circle","x":900,"y":600,"w":900,"h":400})).unwrap(); clamp(&mut s,&m);
  assert_eq!(s["w"],120.); assert_eq!(s["h"],120.); assert_eq!(s["x"],40.); assert_eq!(s["y"],0.);
  let m=DesktopMonitor{width:1280,height:900,scale:1.,..m};
  for (shape,w,h) in [("capsule",320.,144.),("ticket",288.,168.),("bubble",248.,200.),("hexagon",288.,240.),("polaroid",248.,288.),("split",320.,168.)] {
    let mut s=normalize(&json!({"id":1,"kind":"weather","shape":shape,"w":1,"h":1})).unwrap(); clamp(&mut s,&m); assert_eq!(s["w"],w); assert_eq!(s["h"],h);
  }
}
`);
const result = spawnSync('cargo', ['test','--offline','--manifest-path',path.join(out,'Cargo.toml')], { cwd: root, stdio: 'inherit' });
if (result.error) throw result.error;
process.exitCode=result.status ?? 1;
