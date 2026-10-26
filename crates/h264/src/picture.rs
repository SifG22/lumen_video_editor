//! Picture buffers and per-macroblock state.
//!
//! Samples are stored as `u8` for 8-bit pictures and as `u16` for higher bit depths ([`Sample`]);
//! the macroblock layer is generic over the storage type, so 8-bit streams keep their compact,
//! fast path.

use crate::Plane;
use std::sync::{Arc, OnceLock};

/// Storage type of decoded samples: `u8` for 8-bit pictures, `u16` for higher bit depths. Values
/// are always within `0..1 << bit_depth`.
pub trait Sample: Copy + Default + PartialEq + std::fmt::Debug + Send + Sync + 'static {
    fn to_i32(self) -> i32;
    /// `v` (already clipped to the bit depth) in the storage type.
    fn from_i32(v: i32) -> Self;
    /// The rows of `f` if they are stored with this type.
    fn rows(f: &Frame) -> Option<&[OnceLock<FrameRow<Self>>]>;
    fn wrap_rows(rows: Box<[OnceLock<FrameRow<Self>>]>) -> Rows;
    /// An output plane for `n` samples (empty, with room for them).
    fn alloc(n: usize) -> Vec<Self>;
    fn wrap_plane(v: Vec<Self>) -> Plane;
}

impl Sample for u8 {
    #[inline(always)]
    fn to_i32(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u8
    }
    #[inline(always)]
    fn rows(f: &Frame) -> Option<&[OnceLock<FrameRow<u8>>]> {
        match &f.rows {
            Rows::U8(r) => Some(r),
            Rows::U16(_) => None,
        }
    }
    fn wrap_rows(rows: Box<[OnceLock<FrameRow<u8>>]>) -> Rows {
        Rows::U8(rows)
    }
    fn alloc(n: usize) -> Vec<u8> {
        plane_allocator()(n)
    }
    fn wrap_plane(v: Vec<u8>) -> Plane {
        Plane::U8(v)
    }
}

impl Sample for u16 {
    #[inline(always)]
    fn to_i32(self) -> i32 {
        self as i32
    }
    #[inline(always)]
    fn from_i32(v: i32) -> Self {
        v as u16
    }
    #[inline(always)]
    fn rows(f: &Frame) -> Option<&[OnceLock<FrameRow<u16>>]> {
        match &f.rows {
            Rows::U16(r) => Some(r),
            Rows::U8(_) => None,
        }
    }
    fn wrap_rows(rows: Box<[OnceLock<FrameRow<u16>>]>) -> Rows {
        Rows::U16(rows)
    }
    fn alloc(n: usize) -> Vec<u16> {
        Vec::with_capacity(n)
    }
    fn wrap_plane(v: Vec<u16>) -> Plane {
        Plane::U16(v)
    }
}

/// Sample format of a picture: chroma sampling and bit depths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    /// 4:2:2 (else 4:2:0).
    pub c422: bool,
    pub bd_y: u32,
    pub bd_c: u32,
}

impl Format {
    /// Chroma lines per macroblock (MbHeightC).
    #[inline(always)]
    pub fn mb_hc(self) -> usize {
        if self.c422 { 16 } else { 8 }
    }
    /// log2 of [`Format::mb_hc`].
    #[inline(always)]
    pub fn log2_mb_hc(self) -> u32 {
        if self.c422 { 4 } else { 3 }
    }
    /// Samples are stored as `u8`.
    pub fn eight_bit(self) -> bool {
        self.bd_y <= 8 && self.bd_c <= 8
    }
}

/// Planar sample buffers (MB-aligned dimensions, stride == width).
#[derive(Clone)]
pub struct Planes<S> {
    pub y: Vec<S>,
    pub cb: Vec<S>,
    pub cr: Vec<S>,
    pub width: usize,
    pub cwidth: usize,
    pub fmt: Format,
}

impl<S: Sample> Planes<S> {
    pub fn new(width: usize, height: usize, fmt: Format) -> Self {
        let (cw, ch) = (width / 2, height * fmt.mb_hc() / 16);
        let mid = S::from_i32(1 << (fmt.bd_c - 1));
        Planes { y: vec![S::default(); width * height], cb: vec![mid; cw * ch], cr: vec![mid; cw * ch], width, cwidth: cw, fmt }
    }
    pub fn gray(width: usize, height: usize, fmt: Format) -> Self {
        let mut p = Self::new(width, height, fmt);
        p.y.fill(S::from_i32(1 << (fmt.bd_y - 1)));
        p
    }
}

/// Macroblock coding category.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum MbKind {
    #[default]
    None,
    I4x4,
    I8x8,
    I16x16,
    IPcm,
    PSkip,
    BSkip,
    BDirect16x16,
    /// Any other P or B inter macroblock.
    Inter,
}

impl MbKind {
    #[inline]
    pub fn is_intra(self) -> bool {
        matches!(self, MbKind::I4x4 | MbKind::I8x8 | MbKind::I16x16 | MbKind::IPcm)
    }
}

/// Per-macroblock state kept for the whole picture (neighbour derivations, deblocking, co-located data).
#[derive(Clone, Copy)]
pub struct MbState {
    /// Slice number within the picture; `u32::MAX` = not (yet) decoded.
    pub slice_num: u32,
    pub kind: MbKind,
    pub transform_8x8: bool,
    /// coded_block_pattern: bits 0..3 luma, bits 4..5 chroma.
    pub cbp: u8,
    /// QPY (negative for high bit depths).
    pub qp: i8,
    /// QPC for Cb / Cr (used by deblocking).
    pub qpc: [i8; 2],
    pub intra_chroma_mode: u8,
    /// Intra4x4PredMode / Intra8x8PredMode per 4x4 block (raster order).
    pub intra_modes: [u8; 16],
    /// CAVLC: TotalCoeff per luma 4x4 block (raster). CABAC: coded_block_flag.
    pub nnz: [u8; 16],
    /// Same for chroma AC blocks [Cb, Cr][raster 2x2 (4:2:0) or 2x4 (4:2:2)].
    pub nnz_c: [[u8; 8]; 2],
    /// CABAC coded_block_flag of DC blocks: bit0 luma (I16x16), bit1 Cb, bit2 Cr.
    pub cbf_dc: u8,
    /// Luma 4x4 blocks (raster bit) with non-zero coefficients, for deblocking bS = 2.
    pub nz_mask: u16,
    pub ref_idx: [[i8; 4]; 2],
    /// Motion vectors per 4x4 block (raster order).
    pub mv: [[[i16; 2]; 16]; 2],
    /// |mvd| per 4x4 block (raster), saturated, for CABAC context selection.
    pub mvd: [[[u8; 2]; 16]; 2],
    /// Bit per 8x8 block predicted in direct mode.
    pub direct8x8: u8,
}

impl MbState {
    #[inline]
    pub fn kind_is_skip(&self) -> bool {
        matches!(self.kind, MbKind::PSkip | MbKind::BSkip)
    }
}

impl Default for MbState {
    fn default() -> Self {
        MbState {
            slice_num: u32::MAX,
            kind: MbKind::None,
            transform_8x8: false,
            cbp: 0,
            qp: 0,
            qpc: [0; 2],
            intra_chroma_mode: 0,
            intra_modes: [2; 16],
            nnz: [0; 16],
            nnz_c: [[0; 8]; 2],
            cbf_dc: 0,
            nz_mask: 0,
            ref_idx: [[-1; 4]; 2],
            mv: [[[0; 2]; 16]; 2],
            mvd: [[[0; 2]; 16]; 2],
            direct8x8: 0,
        }
    }
}

/// One published macroblock row of a decoded frame: final (deblocked) samples plus the motion data
/// needed as co-located information for direct prediction.
pub struct FrameRow<S> {
    /// 16 luma lines.
    pub y: Box<[S]>,
    /// MbHeightC lines per chroma component.
    pub cb: Box<[S]>,
    pub cr: Box<[S]>,
    /// Per 4x4 block: index mb_x * 16 + raster.
    pub mv: [Box<[[i16; 2]]>; 2],
    /// Per 8x8 block: index mb_x * 4 + b8.
    pub ref_idx: [Box<[i8]>; 2],
    /// Unique id of the referenced picture per 8x8 block (u32::MAX = none).
    pub ref_id: [Box<[u32]>; 2],
    pub intra: Box<[bool]>,
}

/// The macroblock rows of a frame, in its storage type.
pub enum Rows {
    U8(Box<[OnceLock<FrameRow<u8>>]>),
    U16(Box<[OnceLock<FrameRow<u16>>]>),
}

/// The value of a published row (waiting for it with the `threads` feature).
#[inline(always)]
fn wait<T>(cell: &OnceLock<T>) -> Option<&T> {
    #[cfg(feature = "threads")]
    {
        Some(cell.wait())
    }
    #[cfg(not(feature = "threads"))]
    {
        cell.get()
    }
}

/// A decoded (or in-progress) frame as stored in the DPB and referenced by later pictures.
///
/// Rows are published in order by the thread decoding the picture; readers block (per row) until the
/// data they need is available, which lets several pictures decode concurrently.
pub struct Frame {
    pub id: u32,
    pub poc: i32,
    pub width: usize,
    pub height: usize,
    pub cwidth: usize,
    pub cheight: usize,
    pub mb_w: usize,
    pub fmt: Format,
    rows: Rows,
}

/// Motion data of one macroblock row as read for co-located (direct) prediction.
pub struct ColRow<'a> {
    pub mv: &'a [Box<[[i16; 2]]>; 2],
    pub ref_idx: &'a [Box<[i8]>; 2],
    pub ref_id: &'a [Box<[u32]>; 2],
    pub intra: &'a [bool],
}

impl Frame {
    pub fn new(id: u32, poc: i32, mb_w: usize, mb_h: usize, fmt: Format) -> Self {
        fn cells<S: Sample>(n: usize) -> Rows {
            S::wrap_rows((0..n).map(|_| OnceLock::new()).collect())
        }
        let rows = if fmt.eight_bit() { cells::<u8>(mb_h) } else { cells::<u16>(mb_h) };
        Frame { id, poc, width: mb_w * 16, height: mb_h * 16, cwidth: mb_w * 8, cheight: mb_h * fmt.mb_hc(), mb_w, fmt, rows }
    }

    pub fn mb_h(&self) -> usize {
        match &self.rows {
            Rows::U8(r) => r.len(),
            Rows::U16(r) => r.len(),
        }
    }

    /// Publish MB row `r` (ignored if already published, or not of the frame's sample type).
    pub fn publish<S: Sample>(&self, r: usize, row: FrameRow<S>) {
        if let Some(cell) = S::rows(self).and_then(|rows| rows.get(r)) {
            let _ = cell.set(row);
        }
    }

    pub fn is_published(&self, r: usize) -> bool {
        match &self.rows {
            Rows::U8(rows) => rows.get(r).is_some_and(|c| c.get().is_some()),
            Rows::U16(rows) => rows.get(r).is_some_and(|c| c.get().is_some()),
        }
    }

    /// MB row `r` in sample type `S`, waiting until it has been published (None: no such row, or
    /// the frame stores another sample type).
    #[inline]
    pub fn row<S: Sample>(&self, r: usize) -> Option<&FrameRow<S>> {
        wait(S::rows(self)?.get(r)?)
    }

    /// The motion data of MB row `r` (waiting until it has been published).
    pub fn col_row(&self, r: usize) -> Option<ColRow<'_>> {
        fn get<S: Sample>(f: &Frame, r: usize) -> Option<ColRow<'_>> {
            let row = f.row::<S>(r)?;
            Some(ColRow { mv: &row.mv, ref_idx: &row.ref_idx, ref_id: &row.ref_id, intra: &row.intra })
        }
        match &self.rows {
            Rows::U8(_) => get::<u8>(self, r),
            Rows::U16(_) => get::<u16>(self, r),
        }
    }

    /// Wait until the whole frame is available.
    pub fn wait_complete(&self) {
        for r in 0..self.mb_h() {
            let _ = self.col_row(r);
        }
    }

    /// Build a published row from a contiguous plane set (rows of MB row `r`) and motion data.
    pub fn make_row<S: Sample>(planes: &Planes<S>, r: usize, mbs: &[MbState], ref_ids: &dyn Fn(&MbState, usize, i8) -> u32) -> FrameRow<S> {
        let w = planes.width;
        let cw = planes.cwidth;
        let hc = planes.fmt.mb_hc();
        let mb_w = w / 16;
        let y = planes.y[r * 16 * w..(r + 1) * 16 * w].into();
        let cb = planes.cb[r * hc * cw..(r + 1) * hc * cw].into();
        let cr = planes.cr[r * hc * cw..(r + 1) * hc * cw].into();
        let row_mbs = &mbs[r * mb_w..(r + 1) * mb_w];
        let mut mv = [vec![[0i16; 2]; mb_w * 16], vec![[0i16; 2]; mb_w * 16]];
        let mut ref_idx = [vec![-1i8; mb_w * 4], vec![-1i8; mb_w * 4]];
        let mut ref_id = [vec![u32::MAX; mb_w * 4], vec![u32::MAX; mb_w * 4]];
        let mut intra = vec![true; mb_w];
        for (x, st) in row_mbs.iter().enumerate() {
            if st.slice_num == u32::MAX {
                continue;
            }
            intra[x] = st.kind.is_intra();
            for l in 0..2 {
                mv[l][x * 16..x * 16 + 16].copy_from_slice(&st.mv[l]);
                for b in 0..4 {
                    let ri = st.ref_idx[l][b];
                    ref_idx[l][x * 4 + b] = ri;
                    if ri >= 0 {
                        ref_id[l][x * 4 + b] = ref_ids(st, l, ri);
                    }
                }
            }
        }
        let [mv0, mv1] = mv;
        let [ri0, ri1] = ref_idx;
        let [id0, id1] = ref_id;
        FrameRow { y, cb, cr, mv: [mv0.into(), mv1.into()], ref_idx: [ri0.into(), ri1.into()], ref_id: [id0.into(), id1.into()], intra: intra.into() }
    }

    /// Publish every row of the frame not published yet from `planes`, with no motion (intra).
    pub fn fill_from<S: Sample>(&self, planes: &Planes<S>) {
        let blank = vec![MbState::default(); self.mb_w * self.mb_h()];
        for r in 0..self.mb_h() {
            if !self.is_published(r) {
                self.publish(r, Frame::make_row(planes, r, &blank, &|_, _, _| u32::MAX));
            }
        }
    }

    /// Publish the rows not published yet as gray (a picture whose decoding failed: readers never
    /// block on it).
    pub fn fill_gray(&self) {
        match &self.rows {
            Rows::U8(_) => self.fill_from(&Planes::<u8>::gray(self.width, self.height, self.fmt)),
            Rows::U16(_) => self.fill_from(&Planes::<u16>::gray(self.width, self.height, self.fmt)),
        }
    }

    /// A frame with every row published: a copy of `src`'s samples (when it has the same size and
    /// format) without motion, else gray. "Non-existing" frames inferred for frame_num gaps.
    pub fn concealment(id: u32, poc: i32, mb_w: usize, mb_h: usize, fmt: Format, src: Option<&Frame>) -> Self {
        fn copy<S: Sample>(f: &Frame, src: &Frame) -> Option<Planes<S>> {
            let mut p = Planes::<S>::new(f.width, f.height, f.fmt);
            let (w, cw, hc) = (f.width, f.cwidth, f.fmt.mb_hc());
            for r in 0..f.mb_h() {
                let row = src.row::<S>(r)?;
                p.y.get_mut(r * 16 * w..(r + 1) * 16 * w)?.copy_from_slice(&row.y);
                p.cb.get_mut(r * hc * cw..(r + 1) * hc * cw)?.copy_from_slice(&row.cb);
                p.cr.get_mut(r * hc * cw..(r + 1) * hc * cw)?.copy_from_slice(&row.cr);
            }
            Some(p)
        }
        let f = Frame::new(id, poc, mb_w, mb_h, fmt);
        if let Some(src) = src.filter(|s| s.mb_w == mb_w && s.mb_h() == mb_h && s.fmt == fmt) {
            match &f.rows {
                Rows::U8(_) => {
                    if let Some(p) = copy::<u8>(&f, src) {
                        f.fill_from(&p);
                    }
                }
                Rows::U16(_) => {
                    if let Some(p) = copy::<u16>(&f, src) {
                        f.fill_from(&p);
                    }
                }
            }
        }
        f.fill_gray();
        f
    }

    /// Copy the w x h luma window at (x0, y0) (clamped to the picture) into `out` (stride `os`).
    #[inline]
    pub fn luma_window<S: Sample>(&self, x0: i32, y0: i32, w: usize, h: usize, out: &mut [S], os: usize) {
        let (width, height) = (self.width, self.height);
        for r in 0..h {
            let yy = (y0 + r as i32).clamp(0, height as i32 - 1) as usize;
            let Some(row) = self.row::<S>(yy >> 4) else { return };
            let line = &row.y[(yy & 15) * width..(yy & 15) * width + width];
            copy_line::<S, 24>(line, x0, &mut out[r * os..], w);
        }
    }

    /// Copy the w x h window of chroma component `c` at (x0, y0) (clamped) into `out`.
    #[inline]
    pub fn chroma_window<S: Sample>(&self, c: usize, x0: i32, y0: i32, w: usize, h: usize, out: &mut [S], os: usize) {
        let (cw, ch) = (self.cwidth, self.cheight);
        let (sh, mask) = (self.fmt.log2_mb_hc(), self.fmt.mb_hc() - 1);
        for r in 0..h {
            let yy = (y0 + r as i32).clamp(0, ch as i32 - 1) as usize;
            let Some(row) = self.row::<S>(yy >> sh) else { return };
            let plane = if c == 0 { &row.cb } else { &row.cr };
            let line = &plane[(yy & mask) * cw..(yy & mask) * cw + cw];
            copy_line::<S, 16>(line, x0, &mut out[r * os..], w);
        }
    }

    /// Crop and copy the frame into planar buffers (waits for completion); chroma is cropped to
    /// the matching chroma rectangle.
    pub fn copy_cropped(&self, crop: (usize, usize, usize, usize)) -> (Plane, Plane, Plane) {
        fn planes<S: Sample>(f: &Frame, crop: (usize, usize, usize, usize)) -> (Plane, Plane, Plane) {
            let (cx, cy, cw, ch) = crop;
            let mut y = S::alloc(cw * ch);
            for r in cy..cy + ch {
                let Some(row) = f.row::<S>(r >> 4) else { break };
                y.extend_from_slice(&row.y[(r & 15) * f.width + cx..(r & 15) * f.width + cx + cw]);
            }
            let (ccy, cch) = if f.fmt.c422 { (cy, ch) } else { (cy / 2, ch.div_ceil(2)) };
            let (ccx, ccw) = (cx / 2, cw.div_ceil(2));
            let (sh, mask) = (f.fmt.log2_mb_hc(), f.fmt.mb_hc() - 1);
            let mut u = S::alloc(ccw * cch);
            let mut v = S::alloc(ccw * cch);
            for r in ccy..ccy + cch {
                let Some(row) = f.row::<S>(r >> sh) else { break };
                let o = (r & mask) * f.cwidth + ccx;
                u.extend_from_slice(&row.cb[o..o + ccw]);
                v.extend_from_slice(&row.cr[o..o + ccw]);
            }
            (S::wrap_plane(y), S::wrap_plane(u), S::wrap_plane(v))
        }
        match &self.rows {
            Rows::U8(_) => planes::<u8>(self, crop),
            Rows::U16(_) => planes::<u16>(self, crop),
        }
    }
}

static PLANE_ALLOCATOR: std::sync::OnceLock<fn(usize) -> Vec<u8>> = std::sync::OnceLock::new();

/// Where output planes come from: an empty buffer with room for the given number of samples.
/// The host sets it once to recycle the planes of pictures it is done with; the first call wins.
pub fn set_plane_allocator(alloc: fn(usize) -> Vec<u8>) {
    let _ = PLANE_ALLOCATOR.set(alloc);
}

fn plane_allocator() -> fn(usize) -> Vec<u8> {
    PLANE_ALLOCATOR.get().copied().unwrap_or(Vec::with_capacity)
}

/// Copy `w` samples of `line` starting at x0 into `out`; when possible a fixed-size block of `N`
/// samples is copied instead (cheaper than a variable-length copy; `out` must then have room for `N`).
#[inline(always)]
fn copy_line<S: Sample, const N: usize>(line: &[S], x0: i32, out: &mut [S], w: usize) {
    if w <= N
        && x0 >= 0
        && let Some(src) = line.get(x0 as usize..).and_then(|l| l.first_chunk::<N>())
        && let Some(dst) = out.first_chunk_mut::<N>()
    {
        *dst = *src;
    } else {
        copy_clamped(line, x0, &mut out[..w]);
    }
}

/// Copy `out.len()` samples of `line` starting at x0, replicating edge samples outside the line.
#[inline(always)]
fn copy_clamped<S: Sample>(line: &[S], x0: i32, out: &mut [S]) {
    let w = out.len();
    if x0 >= 0 && x0 as usize + w <= line.len() {
        out.copy_from_slice(&line[x0 as usize..x0 as usize + w]);
    } else {
        let max = line.len() as i32 - 1;
        for (c, o) in out.iter_mut().enumerate() {
            *o = line[(x0 + c as i32).clamp(0, max) as usize];
        }
    }
}

pub type FrameRef = Arc<Frame>;

/// An entry of RefPicList0/1 for the current slice.
#[derive(Clone)]
pub struct RefPic {
    pub frame: FrameRef,
    pub long_term: bool,
}

impl RefPic {
    pub fn poc(&self) -> i32 {
        self.frame.poc
    }
    pub fn id(&self) -> u32 {
        self.frame.id
    }
}
