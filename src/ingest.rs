//! Capture / upload / draw all become one ingest path.

use std::path::PathBuf;

use image::RgbaImage;

pub enum IngestSource {
    Screen(RgbaImage),
    Files(Vec<PathBuf>),
    Strokes(Vec<Vec<[f32; 3]>>),
}
