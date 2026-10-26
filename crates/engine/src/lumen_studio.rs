//! Lumen Video Editor's quick creative tools (the **Lumen** menu), the kind consumer editors put a
//! click away:
//!
//! | command | menu | what it does |
//! |---|---|---|
//! | `lumen.panZoom` | Lumen ▸ Pan and Zoom (Ken Burns) | animates Motion ▸ Scale and Position of the selected video clips: a slow zoom with a pan, never showing past the picture's edges |
//! | `lumen.splitScreen` | Lumen ▸ Split Screen | lays out 2–4 selected clips (stacked on different tracks) side by side, top and bottom, one big and two small, or in a 2 × 2 grid; each fills its cell, cropped with Crop |
//! | `lumen.slideshow` | Lumen ▸ Create Slideshow | from the Project panel selection builds a new sequence: the pictures and clips in turn, a cross dissolve at every cut, Pan and Zoom on the stills, and the selected song underneath |
//!
//! Everything is made of ordinary Motion keyframes, Crop effects and transitions, so the result
//! stays editable in Effect Controls. Every command is one undo step.

use lumen_geom::Vec2;
use lumen_project::{ClipId, ItemId, ItemKind, Param, ParamValue, TrackItem, Transition, TransitionId};
use lumen_time::{Tick, TimeRange};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, bad};
use crate::{EngineError, Result, Session};

const MENU: &[&str] = &["Lumen"];

/// Most pictures one slideshow takes.
const MAX_SLIDES: usize = 2000;

pub fn commands() -> Vec<CommandSpec> {
    let spec = |id, label, params, enabled, run| CommandSpec { id, label, menu: MENU, shortcut: None, params, enabled, run, journal: true };
    vec![
        spec(
            "lumen.panZoom",
            "Pan and Zoom (Ken Burns)",
            r#"{"clips":[id]?,"zoom":n=1.25,"direction":"in"|"out"|"alternate"="alternate","pan":"auto"|"none"|"left"|"right"|"up"|"down"="auto"}"#,
            has_video_selection,
            pan_zoom,
        ),
        spec("lumen.splitScreen", "Split Screen", r#"{"clips":[id]?,"layout":"auto"|"2h"|"2v"|"3"|"4"="auto","gap":n=0}"#, has_split_selection, split_screen),
        spec(
            "lumen.slideshow",
            "Create Slideshow",
            r#"{"items":[id]?,"seconds":n=4,"transition":bool=true,"transitionSeconds":n=1,"panZoom":bool=true,"name":str?}"#,
            has_slideshow_selection,
            slideshow,
        ),
    ]
}

// ------------------------------------------------------------------------------------- helpers

/// The video clips a command works on: `clips`, else the selected clips on video tracks, in
/// track order (V1 first), then by start.
fn video_clips(s: &Session, p: &Value) -> Vec<ClipId> {
    let Some(seq) = s.active_sequence() else { return Vec::new() };
    let wanted: Vec<ClipId> = match p.get("clips").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(|v| v.as_u64().map(ClipId)).collect(),
        None => s.state.selection.clone(),
    };
    let mut found: Vec<(usize, Tick, ClipId)> = seq
        .video_tracks
        .iter()
        .enumerate()
        .flat_map(|(ti, t)| t.items.iter().filter(|it| wanted.contains(&it.id)).map(move |it| (ti, it.start, it.id)))
        .collect();
    found.sort();
    found.into_iter().map(|(_, _, c)| c).collect()
}

fn has_video_selection(s: &Session) -> std::result::Result<(), String> {
    crate::commands::has_seq(s)?;
    if video_clips(s, &Value::Null).is_empty() { Err("select video clips in the timeline".into()) } else { Ok(()) }
}

fn has_split_selection(s: &Session) -> std::result::Result<(), String> {
    crate::commands::has_seq(s)?;
    if (2..=4).contains(&video_clips(s, &Value::Null).len()) { Ok(()) } else { Err("select 2 to 4 video clips stacked on different tracks".into()) }
}

/// The picture size of a clip's source, in pixels.
fn picture_size(s: &Session, item: ItemId) -> Option<(u32, u32)> {
    if let Some((_, m, _)) = s.project.resolve_media(item) {
        return m.info.video.as_ref().map(|v| (v.width.max(1), v.height.max(1)));
    }
    match &s.project.item(item)?.kind {
        ItemKind::Sequence(q) => Some((q.settings.width.max(1), q.settings.height.max(1))),
        _ => None,
    }
}

/// Scale (as a factor) that makes a `media`-sized picture cover `w`×`h`.
fn cover(media: (u32, u32), w: f64, h: f64) -> f64 {
    let (mw, mh) = (f64::from(media.0.max(1)), f64::from(media.1.max(1)));
    (w.max(1.0) / mw).max(h.max(1.0) / mh)
}

/// A clip's Motion effect, added when the clip has none.
fn motion(it: &mut TrackItem) -> Option<&mut lumen_project::EffectInstance> {
    if it.effect_mut("motion").is_none() {
        it.effects.insert(0, crate::presets::effect_instance("motion"));
    }
    it.effect_mut("motion")
}

/// A fresh, static parameter `id` of `e` holding `v` (any keyframes go).
fn set_static(e: &mut lumen_project::EffectInstance, id: &str, v: ParamValue) {
    e.params.insert(id.into(), Param::new(v));
}

// ------------------------------------------------------------------------------- Pan and Zoom

/// Where the view travels while zooming.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Pan {
    None,
    Left,
    Right,
    Up,
    Down,
}

impl Pan {
    /// The picture's offset from centre at the zoomed end, as a fraction of the room there is:
    /// the view travels right, so the picture moves left.
    fn offset(self) -> (f64, f64) {
        match self {
            Pan::None => (0.0, 0.0),
            Pan::Left => (1.0, 0.0),
            Pan::Right => (-1.0, 0.0),
            Pan::Up => (0.0, 1.0),
            Pan::Down => (0.0, -1.0),
        }
    }
}

/// How far into the room a pan goes, so the edge never shows even with rounding.
const PAN_REACH: f64 = 0.9;

/// The (scale %, position) pairs at the start and at the end of a Ken Burns move for a
/// `media`-sized picture in `frame`: from covering the frame to `zoom` × that (or back for a
/// zoom out), the view travelling `pan`. Between the two the picture always covers the frame:
/// the room grows with the scale at least as fast as the pan uses it.
pub(crate) fn ken_burns(media: (u32, u32), frame: (u32, u32), zoom: f64, zoom_in: bool, pan: Pan) -> [(f64, Vec2); 2] {
    let (fw, fh) = (f64::from(frame.0.max(1)), f64::from(frame.1.max(1)));
    let base = cover(media, fw, fh);
    let near = base * zoom.clamp(1.0, 4.0);
    let room = |sc: f64| ((f64::from(media.0) * sc - fw).max(0.0) / 2.0, (f64::from(media.1) * sc - fh).max(0.0) / 2.0);
    let (rx, ry) = room(near);
    let (ox, oy) = pan.offset();
    let centre = Vec2::new(fw / 2.0, fh / 2.0);
    let wide = (base * 100.0, centre);
    let close = (near * 100.0, Vec2::new(centre.x + ox * rx * PAN_REACH, centre.y + oy * ry * PAN_REACH));
    if zoom_in { [wide, close] } else { [close, wide] }
}

/// The pan the `i`-th clip gets with `"pan":"auto"`: across a landscape picture, along a portrait one.
fn auto_pan(i: usize, media: (u32, u32), frame: (u32, u32)) -> Pan {
    let wider = u64::from(media.0) * u64::from(frame.1) >= u64::from(media.1) * u64::from(frame.0);
    match (wider, i % 2) {
        (true, 0) => Pan::Right,
        (true, _) => Pan::Left,
        (false, 0) => Pan::Down,
        (false, _) => Pan::Up,
    }
}

fn pan_zoom(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lumen.panZoom";
    let clips = video_clips(s, p);
    if clips.is_empty() {
        return Err(bad(cmd, "select video clips in the timeline"));
    }
    let zoom = p.get("zoom").and_then(Value::as_f64).filter(|z| z.is_finite()).unwrap_or(1.25).clamp(1.01, 4.0);
    let direction = p.get("direction").and_then(Value::as_str).unwrap_or("alternate");
    let pan = match p.get("pan").and_then(Value::as_str).unwrap_or("auto") {
        "auto" => None,
        "none" => Some(Pan::None),
        "left" => Some(Pan::Left),
        "right" => Some(Pan::Right),
        "up" => Some(Pan::Up),
        "down" => Some(Pan::Down),
        other => return Err(bad(cmd, format!("unknown pan `{other}`"))),
    };
    if !matches!(direction, "in" | "out" | "alternate") {
        return Err(bad(cmd, format!("unknown direction `{direction}`")));
    }
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let frame = (seq.settings.width, seq.settings.height);
    // (clip, picture size) for every clip with a picture
    let sized: Vec<(ClipId, (u32, u32))> = clips
        .iter()
        .filter_map(|c| seq.find_item(*c).map(|(_, it)| it.item))
        .zip(clips.iter())
        .filter_map(|(item, c)| picture_size(s, item).map(|sz| (*c, sz)))
        .collect();
    if sized.is_empty() {
        return Err(bad(cmd, "the selected clips have no picture"));
    }
    let n = sized.len();
    s.edit_sequence("Pan and Zoom", |q, _, _| {
        for (i, (c, size)) in sized.iter().enumerate() {
            let zoom_in = match direction {
                "in" => true,
                "out" => false,
                _ => i % 2 == 0,
            };
            let moves = ken_burns(*size, frame, zoom, zoom_in, pan.unwrap_or_else(|| auto_pan(i, *size, frame)));
            let Some((_, it)) = q.find_item_mut(*c) else { continue };
            let last = (it.end() - Tick(1)).max(it.start);
            let times = [it.source_time_at(it.start), it.source_time_at(last)];
            it.scale_to_frame = false;
            let Some(m) = motion(it) else { continue };
            m.enabled = true;
            let mut scale = Param::new(ParamValue::Float(moves[0].0));
            let mut position = Param::new(ParamValue::Vec2(moves[0].1));
            for (t, (sc, pos)) in times.iter().zip(moves) {
                scale.put_keyframe(*t, ParamValue::Float(sc));
                position.put_keyframe(*t, ParamValue::Vec2(pos));
            }
            m.params.insert("scale".into(), scale);
            m.params.insert("position".into(), position);
            // a uniform zoom: Scale drives both axes
            if let Some(u) = m.params.get_mut("uniform_scale") {
                u.value = ParamValue::Bool(true);
            }
        }
        Ok(())
    })?;
    Ok(json!({"clips": n}))
}

// ------------------------------------------------------------------------------- Split Screen

/// The cells of a split-screen layout as (x, y, w, h) fractions of the frame, in clip order.
pub(crate) fn layout_cells(layout: &str, n: usize) -> Option<Vec<[f64; 4]>> {
    let layout = if layout == "auto" {
        match n {
            2 => "2h",
            3 => "3",
            _ => "4",
        }
    } else {
        layout
    };
    let cells = match layout {
        "2h" => vec![[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 0.5, 1.0]],
        "2v" => vec![[0.0, 0.0, 1.0, 0.5], [0.0, 0.5, 1.0, 0.5]],
        "3" => vec![[0.0, 0.0, 0.5, 1.0], [0.5, 0.0, 0.5, 0.5], [0.5, 0.5, 0.5, 0.5]],
        "4" => vec![[0.0, 0.0, 0.5, 0.5], [0.5, 0.0, 0.5, 0.5], [0.0, 0.5, 0.5, 0.5], [0.5, 0.5, 0.5, 0.5]],
        _ => return None,
    };
    Some(cells)
}

/// Scale %, position and Crop (left, top, right, bottom %) that make a `media`-sized picture fill
/// the cell `(x, y, w, h)` (pixels) exactly, centred.
pub(crate) fn fill_cell(media: (u32, u32), cell: [f64; 4]) -> (f64, Vec2, [f64; 4]) {
    let [x, y, w, h] = cell;
    let sc = cover(media, w, h);
    let (sw, sh) = (f64::from(media.0) * sc, f64::from(media.1) * sc);
    let cx = ((sw - w).max(0.0) / 2.0 / sw.max(1.0) * 100.0).clamp(0.0, 49.0);
    let cy = ((sh - h).max(0.0) / 2.0 / sh.max(1.0) * 100.0).clamp(0.0, 49.0);
    (sc * 100.0, Vec2::new(x + w / 2.0, y + h / 2.0), [cx, cy, cx, cy])
}

fn split_screen(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lumen.splitScreen";
    let clips = video_clips(s, p);
    if !(2..=4).contains(&clips.len()) {
        return Err(bad(cmd, "select 2 to 4 video clips stacked on different tracks"));
    }
    let layout = p.get("layout").and_then(Value::as_str).unwrap_or("auto");
    let cells = layout_cells(layout, clips.len()).ok_or_else(|| bad(cmd, format!("unknown layout `{layout}`")))?;
    if cells.len() < clips.len() {
        return Err(bad(cmd, format!("layout `{layout}` has room for {} clips", cells.len())));
    }
    let seq = s.active_sequence().ok_or(EngineError::NoSequence)?;
    let (fw, fh) = (f64::from(seq.settings.width.max(1)), f64::from(seq.settings.height.max(1)));
    let gap = p.get("gap").and_then(Value::as_f64).filter(|g| g.is_finite()).unwrap_or(0.0).clamp(0.0, fw.min(fh) / 4.0);
    let mut plan = Vec::new();
    for (c, cell) in clips.iter().zip(&cells) {
        let Some((_, it)) = seq.find_item(*c) else { continue };
        let Some(size) = picture_size(s, it.item) else { continue };
        let px = [cell[0] * fw + gap / 2.0, cell[1] * fh + gap / 2.0, (cell[2] * fw - gap).max(1.0), (cell[3] * fh - gap).max(1.0)];
        plan.push((*c, fill_cell(size, px)));
    }
    if plan.len() < 2 {
        return Err(bad(cmd, "at least two of the selected clips need a picture"));
    }
    let n = plan.len();
    s.edit_sequence("Split Screen", |q, _, _| {
        for (c, (scale, pos, crop)) in &plan {
            let Some((_, it)) = q.find_item_mut(*c) else { continue };
            it.scale_to_frame = false;
            if let Some(m) = motion(it) {
                m.enabled = true;
                set_static(m, "scale", ParamValue::Float(*scale));
                set_static(m, "position", ParamValue::Vec2(*pos));
                if let Some(u) = m.params.get_mut("uniform_scale") {
                    u.value = ParamValue::Bool(true);
                }
            }
            if it.effect_mut("crop").is_none() {
                it.effects.push(crate::presets::effect_instance("crop"));
            }
            if let Some(e) = it.effect_mut("crop") {
                e.enabled = true;
                for (k, v) in ["left", "top", "right", "bottom"].iter().zip(crop) {
                    set_static(e, k, ParamValue::Float(*v));
                }
            }
        }
        Ok(())
    })?;
    Ok(json!({"clips": n, "layout": layout}))
}

// ---------------------------------------------------------------------------------- Slideshow

/// The Project panel items a slideshow is made of: (song, pictures and clips).
fn slideshow_items(s: &Session, items: &[ItemId]) -> (Option<ItemId>, Vec<ItemId>) {
    let streams = |i: ItemId| s.project.source_streams(i);
    let music = items.iter().copied().find(|i| matches!(streams(*i), (false, true)));
    let pictures = items.iter().copied().filter(|i| streams(*i).0).take(MAX_SLIDES).collect();
    (music, pictures)
}

fn has_slideshow_selection(s: &Session) -> std::result::Result<(), String> {
    if slideshow_items(s, &s.state.project_selection).1.is_empty() { Err("select pictures or clips in the Project panel".into()) } else { Ok(()) }
}

fn is_still(s: &Session, item: ItemId) -> bool {
    s.project.resolve_media(item).is_some_and(|(_, m, _)| matches!(m.info.kind, lumen_media::MediaKind::Still))
}

fn slideshow(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "lumen.slideshow";
    let items: Vec<ItemId> = match p.get("items").and_then(Value::as_array) {
        Some(a) => a.iter().filter_map(|v| v.as_u64().map(ItemId)).collect(),
        None => s.state.project_selection.clone(),
    };
    let (music, pictures) = slideshow_items(s, &items);
    if pictures.is_empty() {
        return Err(bad(cmd, "select pictures or clips in the Project panel"));
    }
    let num = |k: &str, d: f64, lo: f64, hi: f64| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(d).clamp(lo, hi);
    let seconds = num("seconds", 4.0, 0.5, 600.0);
    let with_transitions = p.get("transition").and_then(Value::as_bool).unwrap_or(true);
    let tr_seconds = num("transitionSeconds", 1.0, 0.1, 10.0).min(seconds / 2.0);
    let with_pan_zoom = p.get("panZoom").and_then(Value::as_bool).unwrap_or(true);
    let name = p.get("name").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| "Slideshow".into());
    // HD unless the first slide is footage: then its size and rate
    let settings = pictures
        .iter()
        .find(|i| !is_still(s, **i))
        .and_then(|i| s.project.resolve_media(*i).map(|(_, m, _)| crate::commands::default_seq_settings_for(m)))
        .unwrap_or_default();
    let n0 = s.history.undo.len();
    let seq = s.edit("Create Slideshow", |pr, st| {
        let id = pr.new_sequence(&name, settings, 1, 1, None);
        st.active_sequence = Some(id);
        if !st.open_sequences.contains(&id) {
            st.open_sequences.push(id);
        }
        Ok(id)
    })?;
    let built = build_slideshow(s, music, &pictures, seconds, with_transitions.then_some(tr_seconds), with_pan_zoom);
    crate::clip_ops::collapse_history(s, n0, "Create Slideshow");
    let (slides, length) = match built {
        Ok(v) => v,
        Err(e) => {
            // leave nothing half-made behind
            let _ = s.execute("edit.undo", json!({}));
            return Err(e);
        }
    };
    s.state.selection.clear();
    s.events.push(crate::Event::OpenSequence(seq));
    s.toast(format!("Slideshow: {slides} slides, {:.0} s", length.seconds()));
    Ok(json!({"sequence": seq.0, "slides": slides, "seconds": length.seconds()}))
}

/// The slides on V1 one after the other (stills `seconds` long, footage whole), cross dissolves
/// of `transition` seconds at the cuts, Pan and Zoom on the stills, the song on A1 cut to length.
fn build_slideshow(
    s: &mut Session,
    music: Option<ItemId>,
    pictures: &[ItemId],
    seconds: f64,
    transition: Option<f64>,
    pan_zoom: bool,
) -> Result<(usize, Tick)> {
    let cmd = "lumen.slideshow";
    let (v1, a1) = {
        let q = s.active_sequence().ok_or(EngineError::NoSequence)?;
        (q.video_tracks.first().map(|t| t.id), q.audio_tracks.first().map(|t| t.id))
    };
    let v1 = v1.ok_or_else(|| bad(cmd, "the new sequence has no video track"))?;
    let rate = s.sequence_rate();
    let slot = rate.snap_nearest(Tick::from_seconds_f64(seconds)).max(rate.frame_duration());
    let mut at = Tick::ZERO;
    // (clip, start, length, still)
    let mut placed: Vec<(ClipId, Tick, Tick, bool)> = Vec::new();
    for item in pictures {
        let Some(range) = crate::clip_ops::item_range(&s.project, *item, slot) else { continue };
        // pictures only: a clip's sound would collide with the song
        let clips = crate::commands::place_item(s, *item, range, at, Some(v1), None, false, "Create Slideshow", None)?;
        if let Some(c) = clips.first() {
            placed.push((*c, at, range.duration, is_still(s, *item)));
        }
        at += range.duration;
    }
    if placed.is_empty() {
        return Err(bad(cmd, "none of the selected items could be placed"));
    }
    let length = at;
    if let Some(tr) = transition {
        let dur = rate.snap_nearest(Tick::from_seconds_f64(tr)).max(rate.frame_duration());
        s.edit_sequence("Create Slideshow", |q, ctx, _| {
            for w in placed.windows(2) {
                let (from, to, cut) = (w[0].0, w[1].0, w[1].1);
                // never longer than either slide
                let d = dur.min(w[0].2).min(w[1].2);
                let tr = Transition {
                    id: TransitionId(0),
                    effect: crate::presets::effect_instance("cross_dissolve"),
                    start: (cut - d.mul_ratio(1, 2)).max(Tick::ZERO),
                    duration: d,
                    from: Some(from),
                    to: Some(to),
                    align: Default::default(),
                    reverse: false,
                };
                lumen_edit::add_transition(q, v1, tr, ctx)?;
            }
            Ok(())
        })?;
    }
    if pan_zoom {
        let stills: Vec<u64> = placed.iter().filter(|(_, _, _, still)| *still).map(|(c, _, _, _)| c.0).collect();
        if !stills.is_empty() {
            s.execute("lumen.panZoom", json!({"clips": stills}))?;
        }
    }
    if let (Some(song), Some(a1)) = (music, a1)
        && let Some(full) = crate::clip_ops::item_range(&s.project, song, Tick::ZERO)
    {
        let len = full.duration.min(length);
        crate::commands::place_item(s, song, TimeRange::new(full.start, len), Tick::ZERO, None, Some(a1), false, "Create Slideshow", None)?;
    }
    Ok((placed.len(), length))
}
