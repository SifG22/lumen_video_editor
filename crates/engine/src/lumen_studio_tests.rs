//! Tests of [`crate::lumen_studio`] (Pan and Zoom, Split Screen, Create Slideshow) on generated
//! pictures.

use std::path::Path;

use lumen_project::{ClipId, ItemId, ParamValue};
use serde_json::json;

use super::*;
use crate::media_test_util::{frame_rgba, tmp_dir};

/// Import a `w`×`h` PNG of one colour; returns its item.
fn import_png(s: &mut Session, dir: &Path, name: &str, w: u32, h: u32, rgb: [u8; 3]) -> ItemId {
    let p = dir.join(name);
    image::RgbaImage::from_pixel(w, h, image::Rgba([rgb[0], rgb[1], rgb[2], 255])).save(&p).unwrap();
    let r = s.execute("file.import", json!({"paths": [p.to_string_lossy()]})).unwrap();
    ItemId(r["items"][0].as_u64().unwrap_or_else(|| panic!("{r}")))
}

fn place(s: &mut Session, item: ItemId, track: &str) -> ClipId {
    let r = s.execute("timeline.place", json!({"item": item.0, "track": track, "seconds": 0.0, "audio": false})).unwrap();
    ClipId(r["clips"][0].as_u64().unwrap())
}

fn motion_param(s: &Session, c: ClipId, id: &str) -> lumen_project::Param {
    let (_, it) = s.active_sequence().unwrap().find_item(c).unwrap();
    it.effects.iter().find(|e| e.effect == "motion").unwrap().params[id].clone()
}

fn float(v: &ParamValue) -> f64 {
    match v {
        ParamValue::Float(f) => *f,
        other => panic!("{other:?}"),
    }
}

#[test]
fn pan_and_zoom_moves_from_covering_the_frame_to_closer_without_showing_an_edge() {
    let dir = tmp_dir("studio-pan-zoom");
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "kb", "video": 1, "audio": 1})).unwrap();
    let still = import_png(&mut s, &dir, "photo.png", 1600, 1200, [200, 40, 40]);
    let c = place(&mut s, still, "V1");
    s.execute("timeline.select", json!({"clips": [c.0]})).unwrap();
    assert!(s.is_enabled("lumen.panZoom"));
    let r = s.execute("lumen.panZoom", json!({"zoom": 1.5, "direction": "in", "pan": "right"})).unwrap();
    assert_eq!(r["clips"], 1);
    let scale = motion_param(&s, c, "scale");
    let pos = motion_param(&s, c, "position");
    assert_eq!(scale.keyframes.len(), 2);
    // 1600×1200 covers 1920×1080 at 120 %; 1.5 × that at the end
    assert!((float(&scale.keyframes[0].value) - 120.0).abs() < 1e-6);
    assert!((float(&scale.keyframes[1].value) - 180.0).abs() < 1e-6);
    // the view travels right: the picture moves left, but never further than its edge allows
    let ParamValue::Vec2(end) = pos.keyframes[1].value else { panic!() };
    let room = (1600.0 * 1.8 - 1920.0) / 2.0;
    assert!(end.x < 960.0 && 960.0 - end.x <= room, "{end:?}");
    assert!((end.y - 540.0).abs() < 1e-6);
    // one undo step
    s.execute("edit.undo", json!({})).unwrap();
    assert!(!motion_param(&s, c, "scale").is_animated());
    // hostile parameters are refused or clamped, never a panic
    assert!(s.execute("lumen.panZoom", json!({"pan": "sideways"})).is_err());
    s.execute("lumen.panZoom", json!({"zoom": f64::MAX})).unwrap();
    s.execute("lumen.panZoom", json!({"zoom": -3})).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ken_burns_never_shows_past_the_picture_mid_move() {
    use super::lumen_studio::{Pan, ken_burns};
    for (media, frame) in [((1600, 1200), (1920, 1080)), ((1080, 1920), (1920, 1080)), ((4000, 3000), (1080, 1920)), ((1, 1), (1920, 1080))] {
        for pan in [Pan::None, Pan::Left, Pan::Right, Pan::Up, Pan::Down] {
            for zoom_in in [true, false] {
                let [(s0, p0), (s1, p1)] = ken_burns(media, frame, 1.3, zoom_in, pan);
                for k in 0..=10 {
                    let t = f64::from(k) / 10.0;
                    let sc = (s0 + (s1 - s0) * t) / 100.0;
                    let (x, y) = (p0.x + (p1.x - p0.x) * t, p0.y + (p1.y - p0.y) * t);
                    let (hw, hh) = (f64::from(media.0) * sc / 2.0, f64::from(media.1) * sc / 2.0);
                    let (fw, fh) = (f64::from(frame.0), f64::from(frame.1));
                    assert!(x - hw <= 1e-6 && x + hw >= fw - 1e-6 && y - hh <= 1e-6 && y + hh >= fh - 1e-6, "{media:?} {frame:?} {pan:?} {t}");
                }
            }
        }
    }
}

#[test]
fn split_screen_puts_each_clip_in_its_half() {
    let dir = tmp_dir("studio-split");
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "split", "video": 2, "audio": 1, "fps": 25.0})).unwrap();
    let red = import_png(&mut s, &dir, "red.png", 1920, 1080, [220, 0, 0]);
    let blue = import_png(&mut s, &dir, "blue.png", 1920, 1080, [0, 0, 220]);
    let a = place(&mut s, red, "V1");
    let b = place(&mut s, blue, "V2");
    s.execute("timeline.select", json!({"clips": [a.0]})).unwrap();
    assert!(!s.is_enabled("lumen.splitScreen"), "one clip is not a split screen");
    s.execute("timeline.select", json!({"clips": [b.0, a.0]})).unwrap();
    assert!(s.is_enabled("lumen.splitScreen"));
    assert!(s.execute("lumen.splitScreen", json!({"layout": "9"})).is_err());
    s.execute("lumen.splitScreen", json!({})).unwrap();
    // V1 on the left, V2 on the right, each cropped to its half
    let (w, h, px) = frame_rgba(&mut s, 0, 0.25);
    let at = |x: usize, y: usize| &px[(y * w + x) * 4..(y * w + x) * 4 + 3];
    let (left, right) = (at(w / 4, h / 2), at(w * 3 / 4, h / 2));
    assert!(left[0] > 150 && left[2] < 60, "left {left:?}");
    assert!(right[2] > 150 && right[0] < 60, "right {right:?}");
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn layouts_fill_their_cells() {
    use super::lumen_studio::{fill_cell, layout_cells};
    assert_eq!(layout_cells("auto", 2).unwrap().len(), 2);
    assert_eq!(layout_cells("auto", 3).unwrap().len(), 3);
    assert_eq!(layout_cells("auto", 4).unwrap().len(), 4);
    assert!(layout_cells("x", 2).is_none());
    // a 16:9 picture in a half-width cell: covers the height, half of the width cropped away
    let (sc, pos, crop) = fill_cell((1920, 1080), [0.0, 0.0, 960.0, 1080.0]);
    assert!((sc - 100.0).abs() < 1e-9);
    assert!((pos.x - 480.0).abs() < 1e-9 && (pos.y - 540.0).abs() < 1e-9);
    assert!((crop[0] - 25.0).abs() < 1e-9 && crop[1].abs() < 1e-9);
    // degenerate sizes stay finite
    let (sc, _, crop) = fill_cell((0, 0), [0.0, 0.0, 0.0, 0.0]);
    assert!(sc.is_finite() && crop.iter().all(|c| c.is_finite()));
}

#[test]
fn slideshow_builds_a_sequence_with_dissolves_moves_and_the_song() {
    let dir = tmp_dir("studio-slideshow");
    let mut s = Session::default();
    let pics: Vec<ItemId> =
        [[200, 0, 0], [0, 200, 0], [0, 0, 200]].iter().enumerate().map(|(i, c)| import_png(&mut s, &dir, &format!("p{i}.png"), 1600, 1200, *c)).collect();
    let wav = dir.join("song.wav");
    std::fs::write(&wav, crate::voiceover::write_wav_f32(&vec![0.1; 48_000 * 2 * 20], 48_000)).unwrap();
    let r = s.execute("file.import", json!({"paths": [wav.to_string_lossy()]})).unwrap();
    let song = r["items"][0].as_u64().unwrap();
    let mut items: Vec<u64> = pics.iter().map(|i| i.0).collect();
    items.push(song);
    let r = s.execute("lumen.slideshow", json!({"items": items, "seconds": 3})).unwrap();
    assert_eq!(r["slides"], 3, "{r}");
    assert!((r["seconds"].as_f64().unwrap() - 9.0).abs() < 0.05, "{r}");
    let q = s.active_sequence().unwrap();
    let v1 = &q.video_tracks[0];
    assert_eq!(v1.items.len(), 3);
    assert_eq!(v1.transitions.len(), 2);
    assert!(v1.items.iter().all(|it| it.effects.iter().any(|e| e.effect == "motion" && e.params["scale"].is_animated())));
    // the song, cut to the slideshow's length
    let a1 = &q.audio_tracks[0];
    assert_eq!(a1.items.len(), 1);
    assert!((a1.items[0].duration.seconds() - 9.0).abs() < 0.05);
    // one undo step takes it all away
    let seq = ItemId(r["sequence"].as_u64().unwrap());
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.project.sequence(seq).is_none());
    // nothing to show: refused
    assert!(s.execute("lumen.slideshow", json!({"items": [song]})).is_err());
    // hostile numbers are clamped
    s.execute("lumen.slideshow", json!({"items": [pics[0].0], "seconds": -1, "transitionSeconds": 1e300})).unwrap();
    std::fs::remove_dir_all(dir).unwrap();
}
