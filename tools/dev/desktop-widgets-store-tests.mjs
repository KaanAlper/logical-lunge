// Compile only the real Rust normalization/store functions. No Tauri build,
// desktop process, installed data, or native UI is touched.
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { spawnSync } from 'node:child_process';
const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const source = fs.readFileSync(path.join(root, 'shell/packages/desktop/src/desktop_widgets.rs'), 'utf8');
const functions = source.slice(source.indexOf('fn sizes('), source.indexOf('fn read_store('));
const out = path.join(root, 'build/tests/desktop-widgets-store');
fs.mkdirSync(path.join(out, 'src'), { recursive: true });
fs.writeFileSync(path.join(out, 'Cargo.toml'), '[package]\nname="desktop-widgets-store-tests"\nversion="0.0.0"\nedition="2021"\n[workspace]\n[dependencies]\nserde_json="1"\n');
fs.writeFileSync(path.join(out, 'src/lib.rs'), `#![cfg(test)]\nuse serde_json::{json, Value};\n${functions}\n
#[cfg(test)] mod tests {
  use super::*;
  fn location() -> Value { json!({"countryCode":"TR","country":"Türkiye","city":"İstanbul","district":"Kadıköy","latitude":40.99,"longitude":29.03,"cityLatitude":41.01,"cityLongitude":28.98}) }
  #[test] fn old_defaults_and_legacy_city() {
    let s = parse_store(r#"{"widgets":[{"id":1,"kind":"weather","city":"İzmir"}]}"#).unwrap();
    let w = &s["widgets"][0];
    assert_eq!(w["city"], "İzmir"); assert_eq!(w["appearance"], "standard");
    assert_eq!(w["shape"], "card");
    assert_eq!(w["backgroundOpacity"], 1.); assert_eq!(w["contentOpacity"], 1.);
    assert_eq!(w["location"], Value::Null); assert_eq!(w["recentLocations"], json!([]));
  }
  #[test] fn every_style_roundtrips_independent_opacity_and_locations() {
    for style in ["standard","transparent","outline","glass","futuristic","cartoon","paper","pixel"] {
      let s = parse_store(&json!({"widgets":[{"id":1,"kind":"weather","appearance":style,"backgroundOpacity":0.23,"contentOpacity":0.79,"location":location(),"recentLocations":[location()]}]}).to_string()).unwrap();
      let w = &s["widgets"][0]; assert_eq!(w["appearance"], style); assert_eq!(w["backgroundOpacity"], 0.23); assert_eq!(w["contentOpacity"], 0.79);
      assert_eq!(w["location"], location()); assert_eq!(w["recentLocations"], json!([location()])); assert_eq!(parse_store(&s.to_string()).unwrap(),s);
    }
  }
  #[test] fn invalid_places_and_unknown_style_do_not_drop_store() {
    let mut bad = location(); bad["latitude"] = Value::Null;
    let s = parse_store(&json!({"widgets":[{"id":1,"kind":"weather","appearance":"future","backgroundOpacity":-1,"contentOpacity":9,"location":bad,"recentLocations":[bad,location()]},{"id":2,"kind":"clock"}]}).to_string()).unwrap();
    assert_eq!(s["widgets"].as_array().unwrap().len(),2);
    let w = &s["widgets"][0]; assert_eq!(w["appearance"],"standard"); assert_eq!(w["backgroundOpacity"],0.); assert_eq!(w["contentOpacity"],1.); assert!(w["location"].is_null()); assert_eq!(w["recentLocations"],json!([location()]));
    for (key,value) in [("latitude",json!(91)),("longitude",json!(181)),("cityLatitude",json!("41")),("cityLongitude",Value::Null),("countryCode",json!("Türkiye")),("countryCode",json!("ΤR")),("city",json!(""))] {
      let mut bad=location(); bad[key]=value; assert!(normalize(&json!({"id":1,"kind":"weather","location":bad})).unwrap()["location"].is_null());
    }
  }
  #[test] fn country_codes_normalize_and_recents_deduplicate_before_limit() {
    let mut lower=location(); lower["countryCode"]=json!("tr");
    let mut recent=vec![location(),lower.clone()];
    for i in 0..10 { let mut place=location(); place["district"]=json!(format!("District {i}")); recent.push(place); }
    let s=normalize(&json!({"id":1,"kind":"weather","location":lower,"recentLocations":recent})).unwrap();
    assert_eq!(s["location"]["countryCode"],"TR"); assert_eq!(s["recentLocations"].as_array().unwrap().len(),8);
    assert_eq!(s["recentLocations"][1]["district"],"District 0"); assert_eq!(s["recentLocations"][7]["district"],"District 6");
  }
  #[test] fn every_shape_roundtrips_and_unknown_falls_back() {
    for shape in ["card","capsule","circle","ticket","bubble","hexagon","polaroid","split"] {
      let s=normalize(&json!({"id":1,"kind":"weather","w":288,"h":288,"shape":shape,"appearance":"glass","location":location(),"backgroundOpacity":0.2,"contentOpacity":0.8})).unwrap();
      assert_eq!(s["shape"],shape); assert_eq!(s["appearance"],"glass"); assert_eq!(s["location"],location());
      assert_eq!(s["backgroundOpacity"],0.2); assert_eq!(s["contentOpacity"],0.8);
      assert_eq!(parse_store(&json!({"widgets":[s.clone()]}).to_string()).unwrap()["widgets"][0],s);
    }
    assert_eq!(normalize(&json!({"id":1,"kind":"clock","shape":"unknown"})).unwrap()["shape"],"card");
    let s=normalize(&json!({"id":1,"kind":"weather","shape":"circle","w":264,"h":128})).unwrap();
    assert_eq!(s["w"],264.); assert_eq!(s["h"],264.);
  }
  #[test] fn remembered_sizes_roundtrip_and_malformed_entries_are_ignored() {
    let s=normalize(&json!({"id":1,"kind":"weather","shapeSizes":{"card":[248,128],"capsule":[640,192],"ticket":[-1,128],"bubble":[null,128],"pixel":[300,120],"circle":[900000,900000]}})).unwrap();
    assert_eq!(s["shapeSizes"],json!({"card":[248,128],"capsule":[640,192]}));
    assert_eq!(parse_store(&json!({"widgets":[s.clone()]}).to_string()).unwrap()["widgets"][0],s);
  }
}
`);
const result = spawnSync('cargo', ['test', '--offline', '--manifest-path', path.join(out, 'Cargo.toml')], { cwd: root, stdio: 'inherit' });
if (result.error) throw result.error;
process.exitCode = result.status ?? 1;
