//! Clip ▸ Render and Replace and Clip ▸ Restore Unrendered.
//!
//! **Render and Replace** (`clip.renderAndReplace`) renders each selected video clip with
//! everything applied to it (speed, time remapping, Motion, every effect) to a media file (by
//! default ProRes 4444 with alpha, so a picture made smaller or keyed keeps showing the tracks
//! below), imports it and puts it in place of the clip's source: the clip keeps its place, name,
//! links, transitions and Opacity (rendered at 100 % so blend modes and fades stay live), and plays
//! from the file instead of computing its effects every frame. When transitions touch the clip,
//! as much extra media as they need (`handleFrames`) is rendered on either side if the source has it.
//!
//! **Restore Unrendered** (`clip.restoreUnrendered`) puts the original clip back: source, speed,
//! effects and keyframes, with trims made since applied to the original as well. What it needs is
//! kept with the rendered item, in its metadata ([`RENDERED_FROM`]), so it survives saving and
//! reopening the project.
//!
//! Rendered files go to `Rendered Clips` next to the saved project (else the default export
//! folder). Each command is one undo step; undoing leaves the rendered files on disk.

use std::sync::Arc;

use lumen_project::{ClipId, TrackItem, TrackKind};
use lumen_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad};
use crate::{EngineError, Result, Session};

/// The rendered item's metadata key holding the original clip (JSON).
pub const RENDERED_FROM: &str = "Rendered From";

/// The preset Render and Replace uses unless told otherwise.
const DEFAULT_PRESET: &str = "Apple ProRes 4444 with Alpha";

/// Most clips one Render and Replace takes.
const MAX_CLIPS: usize = 500;

/// Most handle frames on either side.
const MAX_HANDLE_FRAMES: i64 = 10 * 60 * 60;

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "clip.renderAndReplace",
            label: "Render and Replace",
            menu: &["Clip"],
            shortcut: None,
            params: r#"{"clips":[id]?,"preset":str="Apple ProRes 4444 with Alpha","handleFrames":n?,"dir":str?}"#,
            enabled: has_video_clips,
            run: render_and_replace,
            journal: true,
        },
        CommandSpec {
            id: "clip.restoreUnrendered",
            label: "Restore Unrendered",
            menu: &["Clip"],
            shortcut: None,
            params: r#"{"clips":[id]?}"#,
            enabled: has_rendered_clips,
            run: restore_unrendered,
            journal: true,
        },
    ]
}

/// The clips a command works on (`clips`, else the selection) that are on video tracks.
fn video_clips(s: &Session, p: &Value) -> Vec<ClipId> {
    let Some(seq) = s.active_sequence() else { return Vec::new() };
    let wanted: Vec<ClipId> = match p.get("clips").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(|v| v.as_u64().map(ClipId)).collect(),
        None => s.state.selection.clone(),
    };
    wanted.into_iter().filter(|c| seq.find_item(*c).is_some_and(|(t, _)| seq.track(t).is_some_and(|t| t.kind == TrackKind::Video))).take(MAX_CLIPS).collect()
}

fn has_video_clips(s: &Session) -> std::result::Result<(), String> {
    crate::commands::has_seq(s)?;
    if video_clips(s, &Value::Null).is_empty() { Err("select video clips in the timeline".into()) } else { Ok(()) }
}

/// The original a rendered clip was made from, and where in the rendered file it starts.
struct Original {
    clip: TrackItem,
    head: Tick,
}

fn original_of(s: &Session, c: ClipId) -> Option<Original> {
    let (_, it) = s.active_sequence()?.find_item(c)?;
    let raw = s.project.item(it.item)?.metadata.get(RENDERED_FROM)?;
    let v: Value = serde_json::from_str(raw).ok()?;
    let clip = serde_json::from_value(v.get("clip")?.clone()).ok()?;
    let head = Tick(v.get("head").and_then(Value::as_i64).unwrap_or(0).max(0));
    Some(Original { clip, head })
}

fn has_rendered_clips(s: &Session) -> std::result::Result<(), String> {
    crate::commands::has_seq(s)?;
    if s.state.selection.iter().any(|c| original_of(s, *c).is_some()) { Ok(()) } else { Err("select clips made with Render and Replace".into()) }
}

/// A clip whose media time runs with the timeline (so extra media around it can be rendered).
fn is_plain(it: &TrackItem) -> bool {
    it.speed == 1.0
        && !it.reverse
        && it.frame_hold.is_none()
        && !it.effects.iter().any(|e| e.effect == "time_remap" && e.enabled && e.params.values().any(|p| p.is_animated()))
}

/// How much media can be rendered before and after the clip, up to `want` on each side.
fn handles(s: &Session, it: &TrackItem, want: Tick) -> (Tick, Tick) {
    if want <= Tick::ZERO || !is_plain(it) {
        return (Tick::ZERO, Tick::ZERO);
    }
    let Some((_, m, _)) = s.project.resolve_media(it.item) else { return (Tick::ZERO, Tick::ZERO) };
    let head = want.min(it.start).min(it.source_in.max(Tick::ZERO));
    let tail =
        if matches!(m.info.kind, lumen_media::MediaKind::Still) { want } else { want.min((m.info.duration - (it.source_in + it.duration)).max(Tick::ZERO)) };
    (head.max(Tick::ZERO), tail.max(Tick::ZERO))
}

/// Where rendered clips go: `dir`, else `Rendered Clips` next to the project, else the default
/// export folder.
fn render_dir(s: &Session, p: &Value) -> String {
    if let Some(d) = p.get("dir").and_then(Value::as_str).filter(|d| !d.trim().is_empty()) {
        return d.to_string();
    }
    match s.path.as_deref().and_then(|x| std::path::Path::new(x).parent()) {
        Some(d) => d.join("Rendered Clips").to_string_lossy().into_owned(),
        None => crate::export_tools::default_export_dir(s).join("Rendered Clips").to_string_lossy().into_owned(),
    }
}

/// A file name for `stem` in `dir` that is not taken yet.
fn free_path(s: &Session, dir: &str, stem: &str, ext: &str) -> String {
    let safe: String = stem.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') { '_' } else { c }).collect();
    let safe = if safe.trim().trim_matches('.').is_empty() { "Clip".to_string() } else { safe.trim().to_string() };
    let base = std::path::Path::new(dir);
    let mut k = 1;
    loop {
        let name = if k == 1 { format!("{safe} Rendered.{ext}") } else { format!("{safe} Rendered {k}.{ext}") };
        let path = base.join(name).to_string_lossy().into_owned();
        if s.services.file_size(&path).is_err() || k > 9999 {
            return path;
        }
        k += 1;
    }
}

fn render_and_replace(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "clip.renderAndReplace";
    let seq_id = s.state.active_sequence.ok_or(EngineError::NoSequence)?;
    let clips = video_clips(s, p);
    if clips.is_empty() {
        return Err(bad(cmd, "select video clips in the timeline"));
    }
    let preset = p.get("preset").and_then(Value::as_str).unwrap_or(DEFAULT_PRESET).to_string();
    let (_, base) = crate::export_tools::settings_from_params(s, &json!({"preset": preset}), cmd)?;
    let ext = base.extension().to_string();
    let rate = s.sequence_rate();
    let fixed_handles = p.get("handleFrames").and_then(Value::as_i64).map(|f| rate.tick_of(f.clamp(0, MAX_HANDLE_FRAMES)));
    let dir = render_dir(s, p);
    let n0 = s.history.undo.len();
    let mut out = Vec::new();
    for c in clips {
        let r = render_one(s, cmd, seq_id, c, &base, &dir, &ext, fixed_handles);
        match r {
            Ok(v) => out.push(v),
            Err(e) => {
                crate::clip_ops::collapse_history(s, n0, "Render and Replace");
                if s.history.undo.len() > n0 {
                    let _ = s.execute("edit.undo", json!({}));
                }
                return Err(e);
            }
        }
    }
    crate::clip_ops::collapse_history(s, n0, "Render and Replace");
    Ok(json!({"rendered": out}))
}

#[allow(clippy::too_many_arguments)]
fn render_one(
    s: &mut Session,
    cmd: &str,
    seq_id: lumen_project::ItemId,
    c: ClipId,
    base: &lumen_export::ExportSettings,
    dir: &str,
    ext: &str,
    fixed_handles: Option<Tick>,
) -> Result<Value> {
    let seq = s.project.sequence(seq_id).ok_or(EngineError::NoSequence)?;
    let (track, it) = seq.find_item(c).ok_or_else(|| bad(cmd, "no such clip"))?;
    let it = it.clone();
    // handles: what the transitions on the clip reach past its ends
    let want = fixed_handles.unwrap_or_else(|| {
        seq.track(track)
            .map(|t| t.transitions.iter().filter(|tr| tr.from == Some(c) || tr.to == Some(c)).map(|tr| tr.duration).max().unwrap_or(Tick::ZERO))
            .unwrap_or(Tick::ZERO)
    });
    let (head, tail) = handles(s, &it, want);
    // the clip alone in a copy of the sequence, extended by its handles, at 100 % opacity
    let mut snapshot = (*s.project).clone();
    let q = snapshot.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
    for t in q.all_tracks_mut() {
        t.transitions.clear();
        t.effects.clear();
        t.muted = false;
        t.solo = false;
        t.enabled = true;
        t.items.retain(|x| x.id == c);
    }
    let range = TimeRange::new(it.start - head, it.duration + head + tail);
    if let Some((_, x)) = q.find_item_mut(c) {
        x.start = range.start;
        x.source_in -= head;
        x.duration = range.duration;
        x.enabled = true;
        if let Some(o) = x.effects.iter_mut().find(|e| e.effect == "opacity") {
            *o = crate::presets::effect_instance("opacity");
        }
    }
    let mut settings = base.clone();
    settings.include_audio = false;
    settings.range = Some(range);
    settings.path = free_path(s, dir, &it.name, ext);
    let path = settings.path.clone();
    let job = crate::export_tools::spawn_export(s, Arc::new(snapshot), seq_id, settings, format!("Render and Replace {}", it.name), true)?;
    if let Some(j) = s.jobs.iter().find(|j| j.id == job) {
        let v = j.to_json();
        if let Some(e) = v["result"]["error"].as_str() {
            return Err(EngineError::Other(format!("render failed: {e}")));
        }
    }
    // the rendered file, in the clip's bin, remembering the original
    let bin = s.project.root.parent_of(it.item).filter(|b| *b != s.project.root.id);
    let mut ip = json!({"paths": [path]});
    if let Some(b) = bin {
        ip["bin"] = json!(b.0);
    }
    let r = s.execute("file.import", ip)?;
    let new_item = r["items"].get(0).and_then(Value::as_u64).map(lumen_project::ItemId).ok_or_else(|| bad(cmd, format!("could not import {path}")))?;
    let record = serde_json::to_string(&json!({"clip": it, "head": head.0})).map_err(|e| EngineError::Other(e.to_string()))?;
    let opacity = it.effects.iter().find(|e| e.effect == "opacity").cloned();
    s.edit("Render and Replace", |pr, _| {
        if let Some(pi) = pr.items.get_mut(&new_item) {
            pi.metadata.insert(RENDERED_FROM.into(), record);
        }
        let q = pr.sequence_mut(seq_id).ok_or(EngineError::NoSequence)?;
        let (_, x) = q.find_item_mut(c).ok_or_else(|| bad(cmd, "the clip is gone"))?;
        x.item = new_item;
        x.source_in = head;
        x.speed = 1.0;
        x.reverse = false;
        x.frame_hold = None;
        x.hold_filters = false;
        x.scale_to_frame = false;
        x.multicam = None;
        x.time_interpolation = Default::default();
        let mut fx = lumen_project::effect::intrinsic_video();
        if let (Some(o), Some(slot)) = (opacity, fx.iter_mut().find(|e| e.effect == "opacity")) {
            *slot = o;
        }
        x.effects = fx;
        Ok(())
    })?;
    Ok(json!({"clip": c.0, "item": new_item.0, "path": path, "handles": [head.0, tail.0]}))
}

fn restore_unrendered(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "clip.restoreUnrendered";
    let wanted: Vec<ClipId> = match p.get("clips").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(|v| v.as_u64().map(ClipId)).collect(),
        None => s.state.selection.clone(),
    };
    let originals: Vec<(ClipId, Original)> = wanted.into_iter().filter_map(|c| original_of(s, c).map(|o| (c, o))).collect();
    if originals.is_empty() {
        return Err(bad(cmd, "select clips made with Render and Replace"));
    }
    let n = originals.len();
    s.edit_sequence("Restore Unrendered", |q, _, _| {
        for (c, o) in originals {
            let Some((_, cur)) = q.find_item_mut(c) else { continue };
            let mut back = o.clip;
            // trims made since move the original's source in the same way
            let moved = cur.source_in - o.head;
            if is_plain(&back) {
                back.source_in += moved;
            }
            // the clip keeps its place, links and what was done to it since
            back.id = cur.id;
            back.start = cur.start;
            back.duration = cur.duration;
            back.link = cur.link;
            back.group = cur.group;
            back.label = cur.label;
            back.name = cur.name.clone();
            back.enabled = cur.enabled;
            back.markers = cur.markers.clone();
            let opacity = cur.effects.iter().find(|e| e.effect == "opacity").cloned();
            if let (Some(op), Some(slot)) = (opacity, back.effects.iter_mut().find(|e| e.effect == "opacity")) {
                *slot = op;
            }
            // effects added to the rendered clip go on after the original's own
            back.effects.extend(cur.effects.iter().filter(|e| !matches!(e.effect.as_str(), "motion" | "opacity" | "time_remap")).cloned());
            *cur = back;
        }
        Ok(())
    })?;
    Ok(json!({"restored": n}))
}
