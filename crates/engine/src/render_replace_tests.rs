//! Tests of [`crate::render_replace`]: a scaled-down picture over another one, rendered and
//! replaced, looks the same, and Restore Unrendered brings the original back.

use std::path::Path;

use lumen_project::{ClipId, ItemId, ParamValue};
use serde_json::json;

use super::*;
use crate::media_test_util::{frame_rgba, psnr, tmp_dir};

fn import_png(s: &mut Session, dir: &Path, name: &str, rgb: [u8; 3]) -> ItemId {
    let p = dir.join(name);
    image::RgbaImage::from_fn(160, 90, |x, y| image::Rgba([rgb[0] ^ (x as u8 & 0x40), rgb[1], rgb[2] ^ (y as u8 & 0x20), 255])).save(&p).unwrap();
    let r = s.execute("file.import", json!({"paths": [p.to_string_lossy()]})).unwrap();
    ItemId(r["items"][0].as_u64().unwrap_or_else(|| panic!("{r}")))
}

fn clip(s: &Session, c: ClipId) -> lumen_project::TrackItem {
    s.active_sequence().unwrap().find_item(c).unwrap().1.clone()
}

#[test]
fn render_and_replace_bakes_the_effects_and_restore_brings_them_back() {
    let dir = tmp_dir("render-replace");
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "rr", "width": 160, "height": 90, "fps": 25.0, "video": 2, "audio": 1})).unwrap();
    let bg = import_png(&mut s, &dir, "bg.png", [0, 0, 200]);
    let fg = import_png(&mut s, &dir, "fg.png", [220, 40, 0]);
    s.execute("timeline.place", json!({"item": bg.0, "track": "V1", "seconds": 0.0, "audio": false})).unwrap();
    let r = s.execute("timeline.place", json!({"item": fg.0, "track": "V2", "seconds": 0.0, "audio": false})).unwrap();
    let c = ClipId(r["clips"][0].as_u64().unwrap());
    // the top picture at half size in the top-left corner, half transparent
    s.execute("timeline.select", json!({"clips": [c.0]})).unwrap();
    s.execute("effects.setParam", json!({"clip": c.0, "effect": "motion", "param": "scale", "value": 50.0})).unwrap();
    s.execute("effects.setParam", json!({"clip": c.0, "effect": "motion", "param": "position", "value": [40.0, 22.5]})).unwrap();
    s.execute("effects.setParam", json!({"clip": c.0, "effect": "opacity", "param": "opacity", "value": 60.0})).unwrap();
    let before = frame_rgba(&mut s, 3, 1.0).2;
    let original = clip(&s, c);

    assert!(s.is_enabled("clip.renderAndReplace"));
    assert!(!s.is_enabled("clip.restoreUnrendered"));
    let r = s.execute("clip.renderAndReplace", json!({"dir": dir.to_string_lossy()})).unwrap();
    let path = r["rendered"][0]["path"].as_str().unwrap().to_string();
    assert!(Path::new(&path).exists(), "{r}");
    let replaced = clip(&s, c);
    assert_ne!(replaced.item, original.item);
    assert_eq!((replaced.start, replaced.duration), (original.start, original.duration));
    // Motion is baked in; Opacity stays live
    let motion = replaced.effects.iter().find(|e| e.effect == "motion").unwrap();
    assert_eq!(motion.params["scale"].value, ParamValue::Float(100.0));
    let opacity = replaced.effects.iter().find(|e| e.effect == "opacity").unwrap();
    assert_eq!(opacity.params["opacity"].value, ParamValue::Float(60.0));
    // and it looks the same (ProRes 4444 is near lossless)
    let after = frame_rgba(&mut s, 3, 1.0).2;
    let q = psnr(&before, &after);
    assert!(q > 34.0, "PSNR {q:.1} dB");

    // Restore Unrendered: the original source and Motion are back
    assert!(s.is_enabled("clip.restoreUnrendered"));
    s.execute("clip.restoreUnrendered", json!({})).unwrap();
    let back = clip(&s, c);
    assert_eq!(back.item, original.item);
    assert_eq!(back.effects, original.effects);
    assert_eq!(back.source_in, original.source_in);

    // each command is one undo step
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(clip(&s, c).item, replaced.item);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(clip(&s, c), original);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn handles_are_rendered_for_transitions_when_the_media_has_them() {
    let dir = tmp_dir("render-replace-handles");
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "rr", "width": 160, "height": 90, "fps": 25.0, "video": 1, "audio": 1})).unwrap();
    let a = import_png(&mut s, &dir, "a.png", [0, 200, 0]);
    let b = import_png(&mut s, &dir, "b.png", [200, 0, 200]);
    let r = s.execute("timeline.place", json!({"items": [a.0, b.0], "track": "V1", "seconds": 0.0, "audio": false})).unwrap();
    let first = ClipId(r["clips"][0].as_u64().unwrap());
    s.execute("sequence.applyVideoTransition", json!({"clip": first.0, "edge": "out", "frames": 10})).unwrap();
    s.execute("timeline.select", json!({"clips": [first.0]})).unwrap();
    let r = s.execute("clip.renderAndReplace", json!({"dir": dir.to_string_lossy()})).unwrap();
    let h = &r["rendered"][0]["handles"];
    // a still has media on both sides, but the clip starts at 0: nothing before it
    assert_eq!(h[0], 0, "{r}");
    assert!(h[1].as_i64().unwrap() > 0, "{r}");
    // an unknown preset is refused before anything is rendered
    assert!(s.execute("clip.renderAndReplace", json!({"preset": "No Such Preset"})).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}
