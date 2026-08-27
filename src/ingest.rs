//! Capture / upload / draw all become one ingest path.

use std::ops::Range;
use std::path::PathBuf;

use image::RgbaImage;

pub enum IngestSource {
    Screen(RgbaImage),
    Files(Vec<PathBuf>),
    Strokes(Vec<Vec<(f32, f32)>>),
    /// Decode is not implemented yet; variant exists so PDF pages share the queue.
    #[allow(dead_code)]
    PdfPages {
        path: PathBuf,
        pages: Range<u32>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_variant_is_available() {
        let src = IngestSource::PdfPages {
            path: PathBuf::from("paper.pdf"),
            pages: 0..2,
        };
        assert!(matches!(src, IngestSource::PdfPages { .. }));
    }
}
