//! Tests of [`crate::lumen_tools`] (the Lumen menu) on generated sound: rhythmic music at 120 BPM
//! and "speech" (a tone) with pauses.

use std::sync::Arc;

use lumen_audio_dsp::remix as dsp;
use lumen_project::{ClipId, ItemId};
use serde_json::json;

use super::*;

const SR: u32 = 48_000;

/// Import planar stereo `m` as `/name.wav`.
fn import(s: &mut Session, name: &str, m: &[Vec<f32>]) -> ItemId {
    let inter: Vec<f32> = (0..m[0].len()).flat_map(|i| [m[0][i], m[1][i]]).collect();
    let bytes: Arc<[u8]> = crate::previews::write_wav_f32(&inter, SR).into();
    crate::commands::import_bytes(s, &format!("/{name}.wav"), bytes, None).unwrap()
}

/// 32 s of music at 120 BPM.
fn music() -> Vec<Vec<f32>> {
    dsp::test_music(SR, 120.0, 16, 3).0
}

/// `parts` of (seconds, tone on): a 220 Hz tone for "speech", silence for pauses.
fn speech(parts: &[(f64, bool)]) -> Vec<Vec<f32>> {
    let mut ch = Vec::new();
    for (secs, on) in parts {
        let n = (secs * f64::from(SR)) as usize;
        let start = ch.len();
        ch.extend((0..n).map(|i| if *on { 0.3 * ((start + i) as f32 * 220.0 * std::f32::consts::TAU / SR as f32).sin() } else { 0.0 }));
    }
    vec![ch.clone(), ch]
}

fn placed(s: &mut Session, item: ItemId) -> ClipId {
    let r = s.execute("timeline.place", json!({"item": item.0, "audioTrack": "A1", "seconds": 0.0})).unwrap();
    let c = ClipId(r["clips"][0].as_u64().unwrap());
    s.execute("timeline.select", json!({"clips": [c.0]})).unwrap();
    c
}

#[test]
fn mark_beats_puts_a_marker_on_every_beat_and_replaces_them() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "beats", "audio": 1, "video": 1})).unwrap();
    let item = import(&mut s, "music", &music());
    placed(&mut s, item);
    assert!(s.is_enabled("lumen.beatMarkers"));
    let r = s.execute("lumen.beatMarkers", json!({})).unwrap();
    let n = r["markers"].as_u64().unwrap();
    assert!((115.0..125.0).contains(&r["bpm"].as_f64().unwrap()), "{r}");
    assert!(n >= 50, "about one marker per half second: {n}");
    let beats = |s: &Session| s.active_sequence().unwrap().markers.iter().filter(|m| m.name.starts_with("♪ ")).count() as u64;
    assert_eq!(beats(&s), n);
    // every 4th beat, again: replaces the markers instead of adding to them
    let r4 = s.execute("lumen.beatMarkers", json!({"every": 4})).unwrap();
    assert_eq!(beats(&s), r4["markers"].as_u64().unwrap());
    assert!(beats(&s) < n);
    // one undo step each
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(beats(&s), n);
}

#[test]
fn remove_silences_cuts_the_pauses_on_every_track() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "talk", "audio": 1, "video": 1})).unwrap();
    let item = import(&mut s, "talk", &speech(&[(2.0, true), (2.0, false), (2.0, true), (0.3, false), (2.0, true), (1.5, false), (1.0, true)]));
    let c = placed(&mut s, item);
    let before = s.active_sequence().unwrap().duration();
    let r = s.execute("lumen.removeSilences", json!({"padSeconds": 0.1})).unwrap();
    // the 2 s and 1.5 s pauses go (less 0.1 s of air on each side); the 0.3 s breath stays
    assert_eq!(r["removed"], 2, "{r}");
    let cut = r["seconds"].as_f64().unwrap();
    assert!((cut - 3.1).abs() < 0.05, "{r}");
    let after = s.active_sequence().unwrap().duration();
    assert!(((before - after).seconds() - cut).abs() < 0.01);
    // the sound is now three pieces, one after the other with no gaps
    let seq = s.active_sequence().unwrap();
    let mut clips: Vec<_> = seq.audio_tracks[0].items.iter().map(|i| (i.start, i.end())).collect();
    clips.sort();
    assert_eq!(clips.len(), 3);
    assert!(clips.windows(2).all(|w| w[0].1 == w[1].0), "{clips:?}");
    // one undo brings it all back
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active_sequence().unwrap().duration(), before);
    assert!(s.active_sequence().unwrap().find_item(c).is_some());
}

#[test]
fn remove_silences_on_continuous_sound_changes_nothing() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "talk", "audio": 1, "video": 1})).unwrap();
    let item = import(&mut s, "tone", &speech(&[(3.0, true)]));
    placed(&mut s, item);
    let undo = s.history.undo.len();
    let r = s.execute("lumen.removeSilences", json!({})).unwrap();
    assert_eq!(r["removed"], 0);
    assert_eq!(s.history.undo.len(), undo, "no empty undo step");
}

#[test]
fn the_tools_explain_what_they_need() {
    let mut s = Session::default();
    s.execute("file.newSequence", json!({"name": "empty"})).unwrap();
    assert!(!s.is_enabled("lumen.beatMarkers"));
    assert!(!s.is_enabled("lumen.removeSilences"));
    assert!(!s.is_enabled("lumen.beatMontage"));
    let e = s.execute("lumen.removeSilences", json!({})).unwrap_err().to_string();
    assert!(e.contains("select a clip with sound"), "{e}");
    let e = s.execute("lumen.beatMontage", json!({"items": []})).unwrap_err().to_string();
    assert!(e.contains("song"), "{e}");
}

#[test]
fn beat_montage_cuts_the_pictures_to_the_music() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let song = import(&mut s, "song", &music());
    // two demo clips with pictures
    let pictures: Vec<u64> =
        s.project.items.values().filter(|i| s.project.source_streams(i.id).0 && i.as_sequence().is_none()).map(|i| i.id.0).take(2).collect();
    assert_eq!(pictures.len(), 2);
    let seqs = s.project.sequences().count();
    let mut items = vec![song.0];
    items.extend(&pictures);
    let r = s.execute("lumen.beatMontage", json!({"items": items, "beatsPerCut": 4})).unwrap();
    assert_eq!(s.project.sequences().count(), seqs + 1);
    let q = s.active_sequence().unwrap();
    let v1 = &q.video_tracks[0].items;
    assert_eq!(v1.len() as u64, r["cuts"].as_u64().unwrap(), "{r}");
    assert!(v1.len() >= 8, "a cut every 2 s over 32 s: {}", v1.len());
    // the pictures alternate, and every cut but the first sits on a detected beat (2 s apart at 120 BPM)
    assert_ne!(v1[0].item, v1[1].item);
    let starts: Vec<f64> = v1.iter().map(|c| c.start.seconds()).collect();
    for w in starts.windows(2).skip(1) {
        assert!((w[1] - w[0] - 2.0).abs() < 0.1, "{starts:?}");
    }
    assert_eq!(q.audio_tracks[0].items.len(), 1, "the song on A1");
    // one undo removes the whole montage
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.project.sequences().count(), seqs);
}

#[test]
fn vertical_copy_makes_a_filled_9_16_sequence() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let src = s.state.active_sequence.unwrap();
    let n = s.project.sequence(src).unwrap().video_tracks.iter().map(|t| t.items.len()).sum::<usize>();
    let r = s.execute("lumen.verticalCopy", json!({})).unwrap();
    let q = s.active_sequence().unwrap();
    assert_eq!((q.settings.width, q.settings.height), (1080, 1920), "{r}");
    assert_ne!(s.state.active_sequence, Some(src));
    assert_eq!(q.video_tracks.iter().map(|t| t.items.len()).sum::<usize>(), n, "every clip copied");
    // the original is untouched
    let o = s.project.sequence(src).unwrap();
    assert_ne!((o.settings.width, o.settings.height), (1080, 1920));
    // one undo removes the copy
    s.execute("edit.undo", json!({})).unwrap();
    assert!(s.project.items.values().all(|i| !i.name.ends_with("(Vertical)")));
}

#[test]
fn export_for_social_queues_every_format_with_its_own_file() {
    let mut s = Session::default();
    s.execute("file.openDemoProject", json!({})).unwrap();
    let dir = std::env::temp_dir().join(format!("lumen-social-{}", std::process::id()));
    let r = s.execute("lumen.exportForSocial", json!({"folder": dir.to_string_lossy(), "start": false})).unwrap();
    assert_eq!(r["queued"].as_array().unwrap().len(), crate::lumen_tools::SOCIAL_EXPORTS.len(), "{r}");
    let list = s.execute("export.queue.list", json!({})).unwrap();
    let paths: Vec<String> = list["items"].as_array().unwrap().iter().filter_map(|i| i["path"].as_str().map(str::to_string)).collect();
    let mut unique = paths.clone();
    unique.sort();
    unique.dedup();
    assert_eq!(unique.len(), crate::lumen_tools::SOCIAL_EXPORTS.len(), "{paths:?}");
}
