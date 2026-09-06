use gpui::{
    App, Bounds, Element, Entity, GlobalElementId, IntoElement, LayoutId, MouseButton, Style,
    Window,
};

use super::main_window::MainWindow;
use super::orig_view::clamp_strip_h;
use super::scroll::attach_capture_mouse;

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

pub(crate) struct WindowDragCatcher {
    pub view: Entity<MainWindow>,
}

impl IntoElement for WindowDragCatcher {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for WindowDragCatcher {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        (window.request_layout(Style::default(), [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<gpui::Pixels>,
        _state: &mut (),
        _window: &mut Window,
        _cx: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        _bounds: Bounds<gpui::Pixels>,
        _request: &mut (),
        _prepaint: &mut (),
        window: &mut Window,
        _cx: &mut App,
    ) {
        let view = self.view.clone();
        attach_capture_mouse(
            window,
            {
                let view = view.clone();
                move |event, window, cx| {
                    if view.read(cx).window_drag.is_none() {
                        return;
                    }
                    view.update(cx, |this, cx| this.apply_window_drag(event, window, cx));
                }
            },
            {
                let view = view.clone();
                move |event, cx| {
                    if event.button != MouseButton::Left {
                        return;
                    }
                    if view.read(cx).window_drag.is_none() {
                        return;
                    }
                    view.update(cx, |this, cx| this.end_window_drag(cx));
                }
            },
        );
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
        assert!(
            WindowDrag::Sidebar {
                start_x: 0.0,
                start_w: 200.0
            }
            .strip_h_for(40.0, 500.0)
            .is_none()
        );
    }
}
