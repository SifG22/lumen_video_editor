//! Lumen Video Editor's own tools (the **Lumen** menu), for creators who cut to music and talk to
//! camera:
//!
//! | command | menu | what it does |
//! |---|---|---|
//! | `lumen.beatMarkers` | Lumen ▸ Mark Beats | analyses the selected music clip and puts a sequence marker on every beat (or every 2nd / 4th) |
//! | `lumen.beatMontage` | Lumen ▸ Beat Montage | from the Project panel selection (one music item and some video or still items) builds a new sequence with the music and a cut every few beats |
//! | `lumen.removeSilences` | Lumen ▸ Remove Silences | finds the pauses in the selected clip's sound and ripple-deletes them on every track (jump cuts) |
//! | `lumen.exportForSocial` | Lumen ▸ Export for Social Media | queues the active sequence in every social format at once (YouTube 16:9, vertical 9:16, portrait 4:5, square 1:1) and starts the queue |
//! | `lumen.verticalCopy` | Lumen ▸ Vertical Copy (9:16) | a copy of the active sequence at 1080×1920 (or `width`×`height`) with every video clip filling the frame, centred |
//!
//! Beats come from the same analysis as Clip ▸ Remix ([`crate::remix`]). Every command is one undo
//! step. Audio is read in blocks, so a long interview never needs its whole sound in memory.

use lumen_project::{ClipId, ItemId, Label, Marker, MarkerId, MarkerKind, ParamValue, TrackId, TrackKind};
use lumen_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad, has_seq};
use crate::{EngineError, Result, Session};

const MENU: &[&str] = &["Lumen"];

/// Beat markers are named with this prefix, so marking again replaces them.
const BEAT_PREFIX: &str = "♪ ";

/// Longest stretch of sound the tools read (the rest of a longer clip is left as it is).
const MAX_ANALYSIS_S: i64 = 3 * 60 * 60;

/// Most cuts one Beat Montage makes.
const MAX_MONTAGE_CUTS: usize = 2000;

/// Most silences one Remove Silences takes out.
const MAX_SILENCES: usize = 5000;

pub fn commands() -> Vec<CommandSpec> {
    let spec = |id, label, params, enabled, run| CommandSpec { id, label, menu: MENU, shortcut: None, params, enabled, run, journal: true };
    vec![
        spec("lumen.beatMarkers", "Mark Beats", r#"{"clip":id?,"every":1|2|4=1}"#, has_audio_selection, beat_markers),
        spec("lumen.beatMontage", "Beat Montage", r#"{"items":[id]?,"beatsPerCut":n=4,"name":str?}"#, has_montage_selection, beat_montage),
        spec("lumen.exportForSocial", "Export for Social Media", r#"{"folder":str?,"start":bool=true}"#, has_seq, export_for_social),
        spec("lumen.verticalCopy", "Vertical Copy (9:16)", r#"{"width":n=1080,"height":n=1920}"#, has_seq, vertical_copy),
        spec(
            "lumen.removeSilences",
            "Remove Silences",
            r#"{"clip":id?,"thresholdDb":n=-42,"minSeconds":n=0.6,"padSeconds":n=0.12}"#,
            has_audio_selection,
            remove_silences,
        ),
    ]
}

// ------------------------------------------------------------------------------------- helpers

/// The audio clip a command works on: `clip`, else the first selected clip on an audio track, else
/// the audio partner linked to the first selected video clip.
fn audio_clip(s: &Session, p: &Value, cmd: &str) -> Result<ClipId> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let on_audio = |c: ClipId| seq.find_item(c).is_some_and(|(t, _)| seq.track(t).is_some_and(|t| t.kind == TrackKind::Audio));
    if let Some(c) = p.get("clip").and_then(Value::as_u64).map(ClipId) {
        return if on_audio(c) { Ok(c) } else { Err(bad(cmd, "`clip` is not a clip on an audio track")) };
    }
    if let Some(c) = s.state.selection.iter().copied().find(|c| on_audio(*c)) {
        return Ok(c);
    }
    let linked =
        s.state.selection.iter().find_map(|c| seq.find_item(*c).and_then(|(_, it)| it.link)).and_then(|link| {
            seq.all_tracks().filter(|t| t.kind == TrackKind::Audio).flat_map(|t| t.items.iter()).find(|it| it.link == Some(link)).map(|it| it.id)
        });
    linked.ok_or_else(|| bad(cmd, "select a clip with sound in the timeline"))
}

fn has_audio_selection(s: &Session) -> std::result::Result<(), String> {
    has_seq(s)?;
    audio_clip(s, &Value::Null, "lumen").map(|_| ()).map_err(|e| e.to_string())
}

/// A clip at normal speed, forwards and without a frame hold: media and timeline time run together.
fn check_plain(s: &Session, c: ClipId, cmd: &str) -> Result<()> {
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = seq.find_item(c).ok_or_else(|| bad(cmd, "no such clip"))?;
    if (it.speed - 1.0).abs() > 1e-9 || it.reverse || it.frame_hold.is_some() {
        return Err(bad(cmd, "the clip plays at another speed, in reverse or as a frame hold: set it back to 100 % first"));
    }
    Ok(())
}

/// Mono RMS levels (dBFS) of `item`'s sound in `[source_in, source_in + dur)`, one per `frame`
/// samples at `sr`, read in blocks.
fn levels_db(s: &Session, item: ItemId, source_in: Tick, dur: Tick, sr: u32, frame: usize, cmd: &str) -> Result<Vec<f32>> {
    let provider = s.media.provider(s.project.clone(), s.services.clone());
    let src = lumen_render::SourceProvider::source(&provider, item).ok_or_else(|| bad(cmd, "the clip's media is offline"))?;
    let m0 = source_in.to_units_floor(sr as i64);
    let len = dur.to_units_floor(sr as i64).clamp(0, MAX_ANALYSIS_S * sr as i64);
    let frame = frame.max(1);
    let block = (frame * 2048) as i64;
    let mut out = Vec::new();
    let mut at = 0i64;
    while at < len {
        let n = block.min(len - at);
        let buf = src.audio(m0 + at, n as usize, sr).map_err(|e| bad(cmd, format!("reading the clip's audio: {e}")))?;
        let k = 1.0 / buf.channels.len().max(1) as f32;
        for f in 0..(n as usize).div_ceil(frame) {
            let (a, b) = (f * frame, ((f + 1) * frame).min(n as usize));
            let mut sum = 0.0f64;
            for i in a..b {
                let v = buf.channels.iter().map(|c| c.get(i).copied().unwrap_or(0.0)).sum::<f32>() * k;
                sum += f64::from(v) * f64::from(v);
            }
            let rms = (sum / (b - a).max(1) as f64).sqrt();
            out.push(if rms > 1e-9 { (20.0 * rms.log10()) as f32 } else { -180.0 });
        }
        at += n;
    }
    Ok(out)
}

/// The silent stretches of a level track: runs of frames below `threshold_db` lasting at least
/// `min_frames`, shrunk by `pad` frames at each end (so words keep their breath), as frame ranges
/// `[start, end)`. A silence at the very start or end is kept to `pad` frames of air as well.
pub fn silent_ranges(levels: &[f32], threshold_db: f32, min_frames: usize, pad: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < levels.len() {
        if levels.get(i).is_some_and(|l| *l < threshold_db) {
            let start = i;
            while levels.get(i).is_some_and(|l| *l < threshold_db) {
                i += 1;
            }
            let (a, b) = (start + pad, i.saturating_sub(pad));
            if i - start >= min_frames.max(1) && b > a {
                out.push((a, b));
            }
        } else {
            i += 1;
        }
    }
    out
}

// --------------------------------------------------------------------------------- Mark Beats

fn beat_markers(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lumen.beatMarkers";
    let c = audio_clip(s, p, cmd)?;
    check_plain(s, c, cmd)?;
    let every = p.get("every").and_then(Value::as_u64).unwrap_or(1).clamp(1, 16) as usize;
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = seq.find_item(c).ok_or_else(|| bad(cmd, "no such clip"))?;
    let (item, start, source_in, dur) = (it.item, it.start, it.source_in, it.duration);
    let a = crate::remix::analysis(s, item, source_in, dur).map_err(|e| bad(cmd, e.to_string()))?;
    let sr = i64::from(a.sample_rate.max(1));
    let times: Vec<Tick> = a.beats.iter().step_by(every).map(|b| start + Tick::from_units(*b, sr)).filter(|t| *t < start + dur).collect();
    let bpm = a.bpm;
    let n = times.len();
    s.edit_sequence("Mark Beats", |q, ctx, _| {
        q.markers.retain(|m| !m.name.starts_with(BEAT_PREFIX));
        for (i, t) in times.iter().enumerate() {
            let name = format!("{BEAT_PREFIX}{}", i * every + 1);
            q.markers.push(Marker {
                id: MarkerId(ctx.alloc()),
                start: *t,
                duration: Tick::ZERO,
                name,
                comment: String::new(),
                kind: MarkerKind::Comment,
                color: Label::Violet,
            });
        }
        q.markers.sort_by_key(|m| m.start);
        Ok(())
    })?;
    s.toast(format!("{n} beat markers ({bpm:.0} BPM)"));
    Ok(json!({"markers": n, "bpm": bpm}))
}

// ------------------------------------------------------------------------------- Beat Montage

/// The Project panel selection split into the music (the first item with sound and no picture) and
/// the pictures (items with video, in selection order).
fn montage_items(s: &Session, items: &[ItemId]) -> (Option<ItemId>, Vec<ItemId>) {
    let streams = |i: ItemId| s.project.source_streams(i);
    let music = items.iter().copied().find(|i| matches!(streams(*i), (false, true)));
    let pictures = items.iter().copied().filter(|i| streams(*i).0).collect();
    (music, pictures)
}

fn selection_items(s: &Session, p: &Value) -> Vec<ItemId> {
    match p.get("items").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(|v| v.as_u64().map(ItemId)).collect(),
        None => s.state.project_selection.clone(),
    }
}

fn has_montage_selection(s: &Session) -> std::result::Result<(), String> {
    match montage_items(s, &s.state.project_selection) {
        (Some(_), v) if !v.is_empty() => Ok(()),
        _ => Err("select a song and some clips in the Project panel".into()),
    }
}

fn beat_montage(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lumen.beatMontage";
    let items = selection_items(s, p);
    let (music, pictures) = montage_items(s, &items);
    let music = music.ok_or_else(|| bad(cmd, "select a song (an audio item) in the Project panel"))?;
    if pictures.is_empty() {
        return Err(bad(cmd, "select some video clips or pictures as well"));
    }
    let per_cut = p.get("beatsPerCut").and_then(Value::as_u64).unwrap_or(4).clamp(1, 64) as usize;
    let settings = s.project.resolve_media(pictures[0]).map(|(_, m, _)| crate::commands::default_seq_settings_for(m)).unwrap_or_default();
    let music_range = crate::clip_ops::item_range(&s.project, music, Tick::ZERO).ok_or_else(|| bad(cmd, "the song has no duration"))?;
    let name = p.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| "Beat Montage".into());
    let n0 = s.history.undo.len();
    let seq = s.edit("Beat Montage", |pr, st| {
        let id = pr.new_sequence(&name, settings, 1, 1, None);
        st.active_sequence = Some(id);
        if !st.open_sequences.contains(&id) {
            st.open_sequences.push(id);
        }
        Ok(id)
    })?;
    let (cuts, bpm) = match build_montage(s, cmd, music, music_range, &pictures, per_cut) {
        Ok(v) => v,
        Err(e) => {
            // leave nothing half-made behind
            crate::clip_ops::collapse_history(s, n0, "Beat Montage");
            let _ = s.execute("edit.undo", json!({}));
            return Err(e);
        }
    };
    crate::clip_ops::collapse_history(s, n0, "Beat Montage");
    s.state.selection.clear();
    s.events.push(crate::Event::OpenSequence(seq));
    s.toast(format!("Beat Montage: {cuts} cuts at {bpm:.0} BPM"));
    Ok(json!({"sequence": seq.0, "cuts": cuts, "bpm": bpm}))
}

/// Music on the first audio track from 0, then the pictures in turn on V1, one cut every
/// `per_cut` beats; each picture continues where it left off the next time it comes round.
fn build_montage(s: &mut Session, cmd: &str, music: ItemId, music_range: TimeRange, pictures: &[ItemId], per_cut: usize) -> Result<(usize, f64)> {
    let (v1, a1): (Option<TrackId>, Option<TrackId>) = {
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        (q.video_tracks.first().map(|t| t.id), q.audio_tracks.first().map(|t| t.id))
    };
    crate::commands::place_item(s, music, music_range, Tick::ZERO, None, a1, false, "Beat Montage", None)?;
    let a = crate::remix::analysis(s, music, music_range.start, music_range.duration).map_err(|e| bad(cmd, e.to_string()))?;
    let sr = i64::from(a.sample_rate.max(1));
    // cut points: the start, every `per_cut`-th beat, the end of the song
    let mut cuts: Vec<Tick> = vec![Tick::ZERO];
    cuts.extend(a.beats.iter().skip(per_cut).step_by(per_cut).map(|b| Tick::from_units(*b, sr)));
    cuts.push(music_range.duration);
    cuts.dedup();
    cuts.truncate(MAX_MONTAGE_CUTS + 1);
    let mut cursor = vec![Tick::ZERO; pictures.len()];
    let mut placed = 0;
    for (k, w) in cuts.windows(2).enumerate() {
        let (at, slot) = (w[0], w[1] - w[0]);
        if slot <= Tick::ZERO {
            continue;
        }
        let i = k % pictures.len();
        let item = pictures[i];
        let Some(full) = crate::clip_ops::item_range(&s.project, item, slot) else { continue };
        // stills last as long as the slot; footage continues from where it stopped, from its
        // start again when what is left is too short
        let mut from = full.start + cursor[i];
        if from + slot > full.end() {
            cursor[i] = Tick::ZERO;
            from = full.start;
        }
        let len = slot.min(full.duration);
        crate::commands::place_item(s, item, TimeRange::new(from, len), at, v1, None, false, "Beat Montage", None)?;
        cursor[i] += len;
        placed += 1;
    }
    Ok((placed, a.bpm))
}

// ----------------------------------------------------------------------------- Remove Silences

fn remove_silences(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lumen.removeSilences";
    let c = audio_clip(s, p, cmd)?;
    check_plain(s, c, cmd)?;
    let num = |k: &str, d: f64, lo: f64, hi: f64| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d).clamp(lo, hi);
    let threshold = num("thresholdDb", -42.0, -90.0, -6.0) as f32;
    let min_s = num("minSeconds", 0.6, 0.1, 30.0);
    let pad_s = num("padSeconds", 0.12, 0.0, 2.0);
    // 10 ms frames at 16 kHz: plenty for speech pauses
    const SR: u32 = 16_000;
    const FRAME: usize = 160;
    let frames_per_s = f64::from(SR) / FRAME as f64;
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (_, it) = seq.find_item(c).ok_or_else(|| bad(cmd, "no such clip"))?;
    let (item, start, source_in, dur) = (it.item, it.start, it.source_in, it.duration);
    let levels = levels_db(s, item, source_in, dur, SR, FRAME, cmd)?;
    let mut ranges = silent_ranges(&levels, threshold, (min_s * frames_per_s).round() as usize, (pad_s * frames_per_s).round() as usize);
    ranges.truncate(MAX_SILENCES);
    let unit = |f: usize| Tick::from_units(f as i64 * FRAME as i64, i64::from(SR));
    let cut: Vec<TimeRange> = ranges
        .iter()
        .map(|(a, b)| TimeRange::new(start + unit(*a), unit(*b) - unit(*a)))
        .filter(|r| r.duration > Tick::ZERO && r.end() <= start + dur)
        .collect();
    if cut.is_empty() {
        s.toast("No silences found".to_string());
        return Ok(json!({"removed": 0, "seconds": 0.0}));
    }
    let removed: Tick = cut.iter().fold(Tick::ZERO, |acc, r| acc + r.duration);
    let n = cut.len();
    s.edit_sequence("Remove Silences", |q, ctx, st| {
        let tracks: Vec<TrackId> = q.all_tracks().map(|t| t.id).collect();
        // from the end backwards, so earlier ranges keep their times
        for r in cut.iter().rev() {
            lumen_edit::extract(q, &tracks, *r, ctx);
            if st.ripple_sequence_markers {
                crate::sequence_tools::ripple_markers(&mut q.markers, r.end(), -r.duration);
            }
        }
        Ok(())
    })?;
    let seconds = removed.seconds();
    s.toast(format!("Removed {n} silences ({seconds:.1} s)"));
    Ok(json!({"removed": n, "seconds": seconds}))
}

// ------------------------------------------------------------------------ Export for Social Media

/// The formats Export for Social Media makes: built-in preset, file name suffix.
pub const SOCIAL_EXPORTS: [(&str, &str); 4] = [
    ("YouTube 1080p Full HD", "YouTube 16x9"),
    ("Social Vertical 1080×1920", "Reels TikTok Shorts 9x16"),
    ("Social Portrait 4:5 1080×1350", "Feed 4x5"),
    ("Social Square 1:1 1080×1080", "Square 1x1"),
];

/// `<folder>/<sequence> - <suffix>`, with characters file systems refuse replaced.
fn social_path(folder: &std::path::Path, sequence: &str, suffix: &str) -> String {
    let safe: String = sequence.chars().map(|c| if c.is_control() || r#"<>:"/\|?*"#.contains(c) { '_' } else { c }).collect();
    let safe = safe.trim().trim_end_matches('.');
    let safe = if safe.is_empty() { "Sequence" } else { safe };
    folder.join(format!("{safe} - {suffix}")).to_string_lossy().into_owned()
}

fn export_for_social(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lumen.exportForSocial";
    let seq = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let name = s.project.item(seq).map(|i| i.name.clone()).unwrap_or_else(|| "Sequence".into());
    let folder = match p.get("folder").and_then(Value::as_str).filter(|f| !f.trim().is_empty()) {
        Some(f) => std::path::PathBuf::from(crate::export_tools::expand_home(f)),
        None => crate::export_tools::default_export_dir(s),
    };
    let mut added = Vec::new();
    for (preset, suffix) in SOCIAL_EXPORTS {
        let r = s.execute("export.queue.add", json!({"sequences": [seq.0], "preset": preset, "path": social_path(&folder, &name, suffix)}))?;
        added.extend(r.get("added").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_u64));
    }
    let start = p.get("start").and_then(Value::as_bool).unwrap_or(true);
    if start && !added.is_empty() {
        s.execute("export.queue.start", json!({}))?;
    }
    let folder_shown = folder.to_string_lossy().into_owned();
    s.toast(format!("{} exports queued in {folder_shown}", added.len()));
    if added.is_empty() {
        return Err(bad(cmd, "nothing was queued"));
    }
    Ok(json!({"queued": added, "folder": folder_shown, "started": start}))
}

// ------------------------------------------------------------------------------ Vertical Copy

fn vertical_copy(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lumen.verticalCopy";
    let src = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let dim = |k: &str, d: u64| p.get(k).and_then(Value::as_u64).unwrap_or(d).clamp(16, u64::from(lumen_project::MAX_FRAME_SIDE)) as u32;
    let new_frame = (dim("width", 1080), dim("height", 1920));
    let seq = s.project.sequence(src).ok_or(EngineError::NoSequence)?;
    let old_frame = (seq.settings.width, seq.settings.height);
    // the picture size of every video clip's media
    let sizes: std::collections::HashMap<ItemId, (u32, u32)> = seq
        .video_tracks
        .iter()
        .flat_map(|t| t.items.iter())
        .filter_map(|it| {
            let (_, m, _) = s.project.resolve_media(it.item)?;
            m.info.video.as_ref().map(|v| (it.item, (v.width, v.height)))
        })
        .collect();
    let name = format!("{} (Vertical)", s.project.item(src).map(|i| i.name.as_str()).unwrap_or("Sequence"));
    let bin = s.project.root.parent_of(src).filter(|b| *b != s.project.root.id);
    let nid = s.edit("Vertical Copy", |pr, st| {
        let it = pr.item(src).cloned().ok_or_else(|| bad(cmd, "no such sequence"))?;
        let nid = pr.add_item(&name, it.label, it.kind, bin);
        let q = pr.sequence_mut(nid).ok_or_else(|| bad(cmd, "no such sequence"))?;
        q.settings.width = new_frame.0;
        q.settings.height = new_frame.1;
        for t in q.video_tracks.iter_mut() {
            for it in t.items.iter_mut() {
                if let Some(size) = sizes.get(&it.item) {
                    refit(it, *size, old_frame, new_frame);
                }
            }
        }
        st.active_sequence = Some(nid);
        if !st.open_sequences.contains(&nid) {
            st.open_sequences.push(nid);
        }
        Ok(nid)
    })?;
    s.state.selection.clear();
    s.events.push(crate::Event::OpenSequence(nid));
    Ok(json!({"sequence": nid.0, "width": new_frame.0, "height": new_frame.1}))
}

/// Scale (Motion ▸ Scale) that makes a `media`-sized picture fill `frame`, as a factor.
fn fill(media: (u32, u32), frame: (u32, u32)) -> f64 {
    let (mw, mh) = (f64::from(media.0.max(1)), f64::from(media.1.max(1)));
    (f64::from(frame.0) / mw).max(f64::from(frame.1) / mh)
}

/// Motion ▸ Scale (percent) for a clip at `s0` % (with Scale to Frame Size `stf`) in `old_frame`
/// so it fills `new_frame`, keeping any zoom it had beyond filling the old frame.
fn refit_scale(s0: f64, stf: bool, media: (u32, u32), old_frame: (u32, u32), new_frame: (u32, u32)) -> f64 {
    let (mw, mh) = (f64::from(media.0.max(1)), f64::from(media.1.max(1)));
    let fit_old = (f64::from(old_frame.0) / mw).min(f64::from(old_frame.1) / mh);
    let shown = s0 / 100.0 * if stf { fit_old } else { 1.0 };
    let zoom = (shown / fill(media, old_frame)).max(1.0);
    fill(media, new_frame) * zoom * 100.0
}

/// Make a clip fill the new frame, centred, keeping any zoom it had beyond filling the old one. A
/// clip whose Scale or Position is animated is left as it is (its moves were made for the old frame).
fn refit(it: &mut lumen_project::TrackItem, media: (u32, u32), old_frame: (u32, u32), new_frame: (u32, u32)) {
    let stf = it.scale_to_frame;
    let Some(m) = it.effect_mut("motion") else { return };
    if m.params.get("scale").is_some_and(|p| p.is_animated()) || m.params.get("position").is_some_and(|p| p.is_animated()) {
        return;
    }
    let s0 = m.params.get("scale").and_then(|p| p.value.as_f64()).unwrap_or(100.0);
    let scale = refit_scale(s0, stf, media, old_frame, new_frame);
    let centre = lumen_geom::Vec2::new(f64::from(new_frame.0) / 2.0, f64::from(new_frame.1) / 2.0);
    m.params.entry("scale".into()).or_insert_with(|| lumen_project::Param::new(ParamValue::Float(100.0))).value = ParamValue::Float(scale);
    m.params.entry("position".into()).or_insert_with(|| lumen_project::Param::new(ParamValue::Vec2(centre))).value = ParamValue::Vec2(centre);
    it.scale_to_frame = false;
}

#[cfg(test)]
mod tests {
    use super::silent_ranges;

    #[test]
    fn social_exports_get_one_safe_file_name_each() {
        let dir = std::path::Path::new("out");
        let names: Vec<String> = super::SOCIAL_EXPORTS.iter().map(|(_, sfx)| super::social_path(dir, "Viaje: día 1/2", sfx)).collect();
        assert!(names.iter().all(|n| n.contains("Viaje_ día 1_2 - ")), "{names:?}");
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), names.len(), "no two formats write the same file");
        assert!(super::social_path(dir, "  ..", "x").ends_with("Sequence - x"));
        // every format is a built-in preset
        for (preset, _) in super::SOCIAL_EXPORTS {
            assert!(lumen_export::presets::find_builtin(preset).is_some(), "{preset}");
        }
    }

    #[test]
    fn a_landscape_picture_fills_a_portrait_frame() {
        // 1920×1080 footage in 1080×1920: the height decides, 1920 / 1080
        assert!((super::fill((1920, 1080), (1080, 1920)) - 1920.0 / 1080.0).abs() < 1e-9);
        let close = |a: f64, b: f64| (a - b).abs() < 1e-6;
        // 4K footage fitted in an HD frame (Scale to Frame Size): fills the vertical frame
        assert!(close(super::refit_scale(100.0, true, (3840, 2160), (1920, 1080), (1080, 1920)), 100.0 * 1920.0 / 2160.0));
        // HD footage at 100 % in HD: the same
        assert!(close(super::refit_scale(100.0, false, (1920, 1080), (1920, 1080), (1080, 1920)), 100.0 * 1920.0 / 1080.0));
        // zoomed in to 150 %: still 1.5 × the fill
        assert!(close(super::refit_scale(150.0, false, (1920, 1080), (1920, 1080), (1080, 1920)), 150.0 * 1920.0 / 1080.0));
        // a 4:3 picture letterboxed in 16:9 is not kept small: it fills too
        assert!(close(super::refit_scale(100.0, true, (1440, 1080), (1920, 1080), (1080, 1920)), 100.0 * 1920.0 / 1080.0));
    }

    #[test]
    fn pauses_long_enough_are_found_and_padded() {
        // speech (-20 dB), a 50-frame pause, speech, a 5-frame breath, speech, a 30-frame tail
        let mut l = vec![-20.0f32; 10];
        l.extend([-60.0; 50]);
        l.extend([-20.0; 10]);
        l.extend([-60.0; 5]);
        l.extend([-20.0; 10]);
        l.extend([-60.0; 30]);
        assert_eq!(silent_ranges(&l, -40.0, 20, 5), vec![(15, 55), (90, 110)]);
        // the short breath stays; a pad longer than half a pause leaves nothing to cut
        assert_eq!(silent_ranges(&l, -40.0, 20, 30), vec![]);
        assert_eq!(silent_ranges(&[], -40.0, 1, 0), vec![]);
        assert_eq!(silent_ranges(&[-60.0; 4], -40.0, 0, 0), vec![(0, 4)], "a zero minimum still needs one frame");
    }
}
