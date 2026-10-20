//! Golden-image tests for Lumen Video Editor's renderer (test-only crate; the library is empty).
//!
//! `tests/golden.rs` builds small procedural projects (no media files), renders one frame of each
//! through the CPU reference compositor (`lumen-render`) and compares it with a committed PNG
//! in `goldens/` using `lumen_testkit::golden`. The same scenes are composited on the GPU
//! (`lumen-gpu`) when an adapter is available and compared with the CPU result.
//!
//! Regenerate references with `LUMEN_BLESS=1 cargo test -p lumen-golden`.
