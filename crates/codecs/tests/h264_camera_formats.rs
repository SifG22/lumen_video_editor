//! 10-bit and 4:2:2 H.264, the formats most mirrorless and cinema cameras record (Sony XAVC S-I /
//! XAVC HS 4:2:2 10-bit, Panasonic, Canon), open with our own decoder on every platform (#626):
//! MOV and MP4 files made with libx264 decode bit-exact with ffmpeg as 16-bit frames with the right
//! chroma layout.

mod common;

use common::*;
use lumen_frame::{Chroma, PixelData};
use lumen_media::FrameRequest;

fn check(name: &str, pix_fmt: &str, profile: &str, chroma: Chroma, bits: u32) {
    let ff = lumen_testkit::require_ffmpeg!();
    let (w, h) = (320usize, 180usize);
    let src = format!("testsrc2=s={w}x{h}:r=25:d=0.6,noise=alls=8:allf=t");
    let Some(file) =
        fixture(&ff, name, &["-f", "lavfi", "-i", &src, "-c:v", "libx264", "-pix_fmt", pix_fmt, "-profile:v", profile, "-x264-params", "bframes=2:keyint=8"])
    else {
        return;
    };
    let raw = ffmpeg_frames(&ff, &file, pix_fmt);
    let (cw, ch) = (w / 2, if chroma == Chroma::C422 { h } else { h / 2 });
    let bps = if bits > 8 { 2 } else { 1 };
    let fsize = (w * h + 2 * cw * ch) * bps;
    let n = raw.len() / fsize;
    assert!(n >= 10, "{name}: {n} frames");
    let src = lumen_codecs::open_bytes(name, bytes(&file)).expect("open");
    let rate = src.info().frame_rate();
    for i in 0..n {
        let f = src.video_frame(FrameRequest::full(rate.tick_of(i as i64))).unwrap_or_else(|e| panic!("{name}: frame {i}: {e}"));
        match &f.data {
            PixelData::Yuv16 { chroma: c, bits: b, .. } => assert_eq!((*c, *b), (chroma, bits), "{name}"),
            PixelData::Yuv8 { chroma: c, .. } => assert_eq!((*c, 8), (chroma, bits), "{name}"),
            _ => panic!("{name}: not a YUV frame"),
        }
        let want = raw_planes(&raw[i * fsize..(i + 1) * fsize], w, h, cw, ch, bps);
        assert_eq!(max_diff(&planes(&f), &want), 0, "{name}: frame {i}");
    }
}

#[test]
fn xavc_style_422_10bit_mov() {
    check("h264_422p10.mov", "yuv422p10le", "high422", Chroma::C422, 10);
}

#[test]
fn high10_420_mp4() {
    check("h264_420p10.mp4", "yuv420p10le", "high10", Chroma::C420, 10);
}

#[test]
fn high422_8bit_mov() {
    check("h264_422p8.mov", "yuv422p", "high422", Chroma::C422, 8);
}
