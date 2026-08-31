use super::orig_view::clamp_strip_h;

pub(crate) enum WindowDrag {
    Strip {
        start_y: f32,
        start_h: f32,
    },
    Source {
        start_x: f32,
        start_pct: f32,
        work_w: f32,
    },
    Sidebar {
        start_x: f32,
        start_w: f32,
    },
}

impl WindowDrag {
    pub fn is_strip(&self) -> bool {
        matches!(self, Self::Strip { .. })
    }

    pub fn persist_on_end(&self) -> bool {
        matches!(self, Self::Strip { .. } | Self::Sidebar { .. })
    }

    pub fn strip_h_for(&self, y: f32, max_h: f32) -> Option<f32> {
        let Self::Strip { start_y, start_h } = self else {
            return None;
        };
        Some(clamp_strip_h(start_h + (y - start_y), max_h))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persist_on_end_only_strip_and_sidebar() {
        let strip = WindowDrag::Strip {
            start_y: 0.0,
            start_h: 160.0,
        };
        let source = WindowDrag::Source {
            start_x: 0.0,
            start_pct: 0.5,
            work_w: 400.0,
        };
        let sidebar = WindowDrag::Sidebar {
            start_x: 0.0,
            start_w: 232.0,
        };
        assert!(strip.persist_on_end());
        assert!(!source.persist_on_end());
        assert!(sidebar.persist_on_end());
    }

    #[test]
    fn strip_h_for_is_relative_to_press() {
        let drag = WindowDrag::Strip {
            start_y: 10.0,
            start_h: 180.0,
        };
        let next = drag.strip_h_for(40.0, 500.0).expect("strip");
        assert!((next - 210.0).abs() < 0.5, "got {next}");
        assert!(WindowDrag::Sidebar {
            start_x: 0.0,
            start_w: 200.0
        }
        .strip_h_for(40.0, 500.0)
        .is_none());
    }
}
