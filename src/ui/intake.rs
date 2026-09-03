use std::sync::Arc;
use std::time::Duration;

use gpui::{
    canvas, div, ease_out_quint, fill, img, point, prelude::*, px, relative, rgb, svg, Animation,
    AnimationExt, AnyElement, Bounds, Pixels, RenderImage, SharedString,
};

use super::theme;
use crate::imgutil::intake_paper_spec;
use crate::state::{IntakeBatch, IntakeCounts, IntakeWork};

const PLATEN_W: f32 = 500.0;
const PLATEN_H: f32 = 330.0;
const DOT_PITCH: f32 = 20.0;
const REEL_W: f32 = 416.0;
const REEL_H: f32 = 118.0;
const CARD_SIZE: f32 = 82.0;
const PROGRESS_W: f32 = 330.0;
pub(crate) const INTAKE_REJECT_HOLD: Duration = Duration::from_millis(700);
pub(crate) const INTAKE_RESULT_HOLD: Duration = Duration::from_millis(480);
pub(crate) const INTAKE_LEAVE: Duration = Duration::from_millis(260);
pub(crate) const INTAKE_COMPLETE_HOLD: Duration = Duration::from_millis(1_500);
pub(crate) const INTAKE_PLATEN_OUT: Duration = Duration::from_millis(280);
const INTAKE_WINDOW: usize = 5;

#[derive(Clone, Copy)]
pub(crate) enum IntakeKind {
    Hover,
    Flash,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntakeVisual {
    Backlog,
    Visible,
    Feedback,
    Leaving,
    Gone,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IntakePhase {
    Running,
    Complete,
    Fading,
}

#[derive(Clone, Debug)]
pub(crate) struct IntakeCard {
    pub key: u64,
    pub id: Option<uuid::Uuid>,
    pub work: IntakeWork,
    pub visual: IntakeVisual,
    pub slot: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct IntakePresentation {
    pub gen: u64,
    pub counts: IntakeCounts,
    pub phase: IntakePhase,
    cards: Vec<IntakeCard>,
    done: usize,
    succeeded: usize,
    failed: usize,
}

impl IntakePresentation {
    pub fn new(batch: &IntakeBatch) -> (Self, Vec<u64>) {
        let mut presentation = Self {
            gen: batch.gen,
            counts: batch.counts(),
            phase: IntakePhase::Running,
            cards: Vec::new(),
            done: 0,
            succeeded: 0,
            failed: 0,
        };
        let feedback = presentation.sync(batch);
        (presentation, feedback)
    }

    pub fn sync(&mut self, batch: &IntakeBatch) -> Vec<u64> {
        debug_assert_eq!(self.gen, batch.gen);
        self.counts = batch.counts();
        let (done, total) = batch.progress();
        let (succeeded, failed) = batch.results();
        self.done = done;
        self.counts.images = total;
        self.succeeded = succeeded;
        self.failed = failed;

        let mut feedback = Vec::new();
        for item in &batch.items {
            if let Some(card) = self.cards.iter_mut().find(|card| card.key == item.key) {
                let became_terminal = !card.work.is_terminal() && item.work.is_terminal();
                card.id = item.id;
                card.work = item.work;
                if became_terminal && card.visual == IntakeVisual::Visible {
                    card.visual = IntakeVisual::Feedback;
                    feedback.push(card.key);
                }
                continue;
            }
            let slot = self.first_vacant_slot();
            let visual = match (slot, item.work.is_terminal()) {
                (Some(_), true) => IntakeVisual::Feedback,
                (Some(_), false) => IntakeVisual::Visible,
                (None, _) => IntakeVisual::Backlog,
            };
            self.cards.push(IntakeCard {
                key: item.key,
                id: item.id,
                work: item.work,
                visual,
                slot,
            });
            if visual == IntakeVisual::Feedback {
                feedback.push(item.key);
            }
        }
        feedback
    }

    fn first_vacant_slot(&self) -> Option<usize> {
        (0..INTAKE_WINDOW).find(|slot| {
            !self
                .cards
                .iter()
                .any(|card| card.slot == Some(*slot) && card.visual != IntakeVisual::Gone)
        })
    }

    pub fn begin_leave(&mut self, key: u64) -> bool {
        let Some(card) = self.cards.iter_mut().find(|card| card.key == key) else {
            return false;
        };
        if card.visual != IntakeVisual::Feedback {
            return false;
        }
        card.visual = IntakeVisual::Leaving;
        true
    }

    pub fn mark_gone_and_promote(&mut self, key: u64) -> Option<u64> {
        let slot = {
            let card = self.cards.iter_mut().find(|card| card.key == key)?;
            if card.visual != IntakeVisual::Leaving {
                return None;
            }
            card.visual = IntakeVisual::Gone;
            card.slot.take()?
        };
        let replacement = self
            .cards
            .iter_mut()
            .find(|card| card.visual == IntakeVisual::Backlog)?;
        replacement.slot = Some(slot);
        replacement.visual = if replacement.work.is_terminal() {
            IntakeVisual::Feedback
        } else {
            IntakeVisual::Visible
        };
        (replacement.visual == IntakeVisual::Feedback).then_some(replacement.key)
    }

    pub fn all_gone(&self) -> bool {
        !self.cards.is_empty()
            && self
                .cards
                .iter()
                .all(|card| card.visual == IntakeVisual::Gone)
    }

    pub fn backlog_count(&self) -> usize {
        self.cards
            .iter()
            .filter(|card| card.visual == IntakeVisual::Backlog)
            .count()
    }

    pub fn progress(&self) -> (usize, usize) {
        (self.done, self.counts.images)
    }

    pub fn results(&self) -> (usize, usize) {
        (self.succeeded, self.failed)
    }

    pub fn status(&self) -> (theme::StatusKind, String) {
        if self.counts.images == 0 {
            return (theme::StatusKind::Error, "No images in this drop".into());
        }
        let (done, total) = self.progress();
        if self.phase != IntakePhase::Running {
            return if self.failed == 0 {
                (theme::StatusKind::Ready, format!("Recognized {total}"))
            } else {
                (
                    theme::StatusKind::Error,
                    format!(
                        "Recognized {} of {total} · {} failed",
                        self.succeeded, self.failed
                    ),
                )
            };
        }
        (
            theme::StatusKind::Busy,
            format!("Recognizing {} of {total}", (done + 1).min(total)),
        )
    }

    pub fn thumb_ids(&self) -> Vec<uuid::Uuid> {
        self.visible_cards()
            .filter_map(|(_, card)| card.id)
            .collect()
    }

    pub fn paper_jobs(&self) -> Vec<(uuid::Uuid, crate::imgutil::IntakePaperSpec)> {
        self.visible_cards()
            .filter_map(|(slot, card)| {
                card.id
                    .map(|id| (id, intake_paper_spec(self.gen, card.key, slot)))
            })
            .collect()
    }

    fn visible_cards(&self) -> impl Iterator<Item = (usize, &IntakeCard)> {
        let mut visible: Vec<_> = self
            .cards
            .iter()
            .filter(|card| card.visual != IntakeVisual::Gone)
            .filter_map(|card| card.slot.map(|slot| (slot, card)))
            .collect();
        visible.sort_by_key(|(slot, _)| *slot);
        visible.into_iter()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct IntakePose {
    dx: f32,
    dy: f32,
}

fn intake_pose(index: usize) -> IntakePose {
    const DX: [f32; 6] = [-165.0, -99.0, -33.0, 33.0, 99.0, 165.0];
    let index = index.min(5);
    IntakePose {
        dx: DX[index],
        dy: 0.0,
    }
}

pub(crate) struct IntakeSlot {
    pub key: SharedString,
    pub work: IntakeWork,
    pub visual: IntakeVisual,
    pub thumb: Option<Arc<RenderImage>>,
    pub slot: usize,
    pub degrees: f32,
}

pub(crate) fn slots_from_batch(
    batch: &IntakePresentation,
    thumbs: impl Fn(uuid::Uuid, bool) -> Option<Arc<RenderImage>>,
) -> Vec<IntakeSlot> {
    batch
        .visible_cards()
        .map(|(slot, item)| {
            let paper = intake_paper_spec(batch.gen, item.key, slot);
            IntakeSlot {
                key: SharedString::from(item.key.to_string()),
                work: item.work,
                visual: item.visual,
                thumb: item
                    .id
                    .and_then(|id| thumbs(id, item.work == IntakeWork::Working)),
                slot,
                degrees: paper.degrees,
            }
        })
        .collect()
}

pub(crate) fn render_intake_overlay(
    kind: IntakeKind,
    counts: IntakeCounts,
    batch: Option<&IntakePresentation>,
    slots: &[IntakeSlot],
) -> impl IntoElement {
    let reject = counts.images == 0;
    let fading = batch.is_some_and(|batch| batch.phase == IntakePhase::Fading);
    let complete = batch.is_some_and(|batch| batch.phase != IntakePhase::Running);
    let (title, sub) = intake_copy(kind, counts, batch);
    let title_color = if reject {
        rgb(theme::DANGER)
    } else {
        rgb(theme::TEXT)
    };
    let copy = div()
        .flex()
        .flex_col()
        .items_center()
        .child(
            div()
                .text_lg()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(title_color)
                .child(title),
        )
        .child(
            div()
                .mt_2()
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .child(sub),
        );
    let progress = batch
        .filter(|batch| batch.counts.images > 0)
        .map(render_progress);
    let reel = batch
        .filter(|batch| batch.phase == IntakePhase::Running)
        .filter(|_| !slots.is_empty())
        .map(|batch| {
            let center_offset = reel_center_offset(slots, batch.backlog_count());
            div()
                .mt_4()
                .relative()
                .w(px(REEL_W))
                .h(px(REEL_H))
                .children(slots.iter().map(|slot| render_slot(slot, center_offset)))
                .when(batch.backlog_count() > 0, |d| {
                    d.child(render_more(batch.backlog_count()))
                })
                .children(
                    slots
                        .iter()
                        .filter_map(|slot| render_slot_check(slot, center_offset)),
                )
        });
    let complete_view = complete.then(|| render_complete(batch.expect("complete batch exists")));
    let platen = div()
        .id("intake-platen")
        .w_full()
        .max_w(px(PLATEN_W))
        .h_full()
        .max_h(px(PLATEN_H))
        .relative()
        .overflow_hidden()
        .rounded_md()
        .border_1()
        .border_color(if reject { rgb(0xf0b4b4) } else { rgb(0xd7d3c8) })
        .shadow_sm()
        .child(
            canvas(
                |_, _, _| {},
                |bounds, _, window, _| {
                    paint_platen_dots(bounds, window);
                },
            )
            .size_full(),
        )
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .px_6()
                .child(copy)
                .children(progress)
                .children(reel)
                .children(complete_view),
        )
        .with_animation(
            if fading {
                "intake-platen-out"
            } else {
                "intake-platen-in"
            },
            Animation::new(Duration::from_millis(if fading { 280 } else { 180 }))
                .with_easing(ease_out_quint()),
            move |this, delta| this.opacity(if fading { 1.0 - delta } else { delta }),
        );

    div()
        .id("intake-overlay")
        .absolute()
        .inset_0()
        .p_4()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme::intake_scrim())
        .child(platen)
}

fn render_progress(batch: &IntakePresentation) -> AnyElement {
    let (done, total) = batch.progress();
    let fraction = if total == 0 {
        0.0
    } else {
        done as f32 / total as f32
    };
    div()
        .mt_3()
        .w(px(PROGRESS_W))
        .h(px(4.))
        .overflow_hidden()
        .rounded_full()
        .bg(rgb(0xe7e7e9))
        .child(
            div()
                .h_full()
                .w(relative(fraction))
                .rounded_full()
                .bg(rgb(theme::ACCENT)),
        )
        .into_any_element()
}

fn reel_center_offset(slots: &[IntakeSlot], backlog: usize) -> f32 {
    if backlog > 0 || slots.is_empty() {
        return 0.0;
    }
    let first = slots
        .iter()
        .map(|slot| intake_pose(slot.slot).dx)
        .fold(f32::INFINITY, f32::min);
    let last = slots
        .iter()
        .map(|slot| intake_pose(slot.slot).dx)
        .fold(f32::NEG_INFINITY, f32::max);
    -(first + last) * 0.5
}

fn render_slot(slot: &IntakeSlot, center_offset: f32) -> AnyElement {
    let pose = intake_pose(slot.slot);
    let waiting = slot.work == IntakeWork::Waiting;
    let working = slot.work == IntakeWork::Working;
    let failed = slot.work == IntakeWork::Failed;
    let leaving = slot.visual == IntakeVisual::Leaving;
    let thumb = slot.thumb.clone();
    let left = REEL_W * 0.5 + pose.dx + center_offset - CARD_SIZE * 0.5;
    let top = REEL_H * 0.5 + pose.dy - CARD_SIZE * 0.5;
    let pane = render_thumb_pane(slot, thumb, working);
    let inner = if leaving {
        div()
            .size_full()
            .child(pane)
            .with_animation(
                SharedString::from(format!("intake-leave-{}", slot.key)),
                Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()),
                |this, delta| this.p(px(delta * 6.)).opacity(1.0 - delta),
            )
            .into_any_element()
    } else {
        div()
            .size_full()
            .child(pane)
            .with_animation(
                SharedString::from(format!("intake-fill-{}", slot.key)),
                Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()),
                |this, delta| this.opacity(delta),
            )
            .into_any_element()
    };
    div()
        .id(SharedString::from(format!("intake-slot-{}", slot.key)))
        .absolute()
        .left(px(left))
        .top(px(top))
        .size(px(CARD_SIZE))
        .when(waiting || failed, |d| d.opacity(0.72))
        .child(inner)
        .into_any_element()
}

fn render_thumb_pane(
    slot: &IntakeSlot,
    thumb: Option<Arc<RenderImage>>,
    working: bool,
) -> AnyElement {
    let pane = div()
        .relative()
        .size_full()
        .when_some(thumb, |d, img_data| {
            d.child(
                img(img_data)
                    .size_full()
                    .object_fit(gpui::ObjectFit::Contain),
            )
        });
    if working {
        pane.with_animation(
            SharedString::from(format!("intake-glow-{}", slot.key)),
            Animation::new(Duration::from_millis(1_050)).repeat(),
            |this, delta| {
                let wave = (delta - 0.5).abs() * 2.0;
                this.opacity(0.72 + 0.28 * wave)
            },
        )
        .into_any_element()
    } else {
        pane.into_any_element()
    }
}

fn render_slot_check(slot: &IntakeSlot, center_offset: f32) -> Option<AnyElement> {
    if slot.work != IntakeWork::Succeeded
        || !matches!(slot.visual, IntakeVisual::Feedback | IntakeVisual::Leaving)
    {
        return None;
    }
    let pose = intake_pose(slot.slot);
    let (check_dx, check_dy) = rotated_check_offset(slot.degrees);
    let left = REEL_W * 0.5 + pose.dx + center_offset + check_dx - 13.0;
    let top = REEL_H * 0.5 + pose.dy + check_dy - 13.0;
    let leaving = slot.visual == IntakeVisual::Leaving;
    Some(
        div()
            .absolute()
            .left(px(left))
            .top(px(top))
            .size(px(26.))
            .rounded_full()
            .border_1()
            .border_color(rgb(theme::ON_ACCENT))
            .bg(rgb(theme::OK))
            .shadow_sm()
            .flex()
            .items_center()
            .justify_center()
            .child(
                svg()
                    .path("icons/check.svg")
                    .size(px(14.))
                    .flex_shrink_0()
                    .text_color(rgb(theme::ON_ACCENT)),
            )
            .with_animation(
                SharedString::from(format!(
                    "intake-check-{}-{}",
                    if leaving { "leave" } else { "show" },
                    slot.key
                )),
                Animation::new(Duration::from_millis(if leaving { 260 } else { 340 }))
                    .with_easing(ease_out_quint()),
                move |this, delta| this.opacity(if leaving { 1.0 - delta } else { delta }),
            )
            .into_any_element(),
    )
}

fn rotated_check_offset(degrees: f32) -> (f32, f32) {
    let radians = degrees.to_radians();
    let (sin, cos) = radians.sin_cos();
    let anchor = (35.0, -35.0);
    (
        cos * anchor.0 - sin * anchor.1,
        sin * anchor.0 + cos * anchor.1,
    )
}

fn render_more(count: usize) -> AnyElement {
    let pose = intake_pose(5);
    let left = REEL_W * 0.5 + pose.dx - CARD_SIZE * 0.5;
    let top = REEL_H * 0.5 + pose.dy - CARD_SIZE * 0.5;
    div()
        .id("intake-more")
        .absolute()
        .left(px(left))
        .top(px(top))
        .size(px(CARD_SIZE))
        .relative()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .font_weight(gpui::FontWeight::SEMIBOLD)
        .child(
            img("icons/intake-stack.svg")
                .absolute()
                .inset_0()
                .size_full(),
        )
        .child(
            div()
                .relative()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .child(format!("+{count}"))
                .child(
                    div()
                        .mt_1()
                        .text_size(px(8.))
                        .font_weight(gpui::FontWeight::MEDIUM)
                        .text_color(rgb(0x85817a))
                        .child("REMAINING"),
                ),
        )
        .into_any_element()
}

fn render_complete(batch: &IntakePresentation) -> AnyElement {
    const W: f32 = 190.0;
    const H: f32 = 118.0;
    const CX: f32 = W * 0.5;
    const CY: f32 = 45.0;
    let gen = batch.gen;
    let (_, failed) = batch.results();
    let label = if failed == 0 {
        "All images recognized".to_string()
    } else if failed == 1 {
        "Finished with 1 failed".to_string()
    } else {
        format!("Finished with {failed} failed")
    };
    let mut effect = div()
        .mt_3()
        .relative()
        .w(px(W))
        .h(px(H))
        .child(complete_ring(gen, 0, CX, CY))
        .child(complete_ring(gen, 1, CX, CY));
    for index in 0..8 {
        effect = effect.child(complete_spark(gen, index, CX, CY));
    }
    effect
        .child(
            div()
                .absolute()
                .left(px(CX - 34.))
                .top(px(CY - 34.))
                .size(px(68.))
                .rounded_full()
                .bg(rgb(theme::OK))
                .shadow_sm()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    svg()
                        .path("icons/check.svg")
                        .size(px(30.))
                        .text_color(rgb(theme::ON_ACCENT)),
                )
                .with_animation(
                    SharedString::from(format!("intake-complete-mark-{gen}")),
                    Animation::new(Duration::from_millis(520)).with_easing(ease_out_quint()),
                    |this, delta| this.opacity(delta),
                ),
        )
        .child(
            div()
                .absolute()
                .bottom(px(0.))
                .w_full()
                .text_center()
                .text_sm()
                .text_color(rgb(theme::MUTED))
                .child(label),
        )
        .into_any_element()
}

fn complete_ring(gen: u64, index: usize, cx: f32, cy: f32) -> AnyElement {
    div()
        .absolute()
        .rounded_full()
        .border_1()
        .border_color(rgb(0x79c990))
        .with_animation(
            SharedString::from(format!("intake-complete-ring-{gen}-{index}")),
            Animation::new(Duration::from_millis(1_000)),
            move |this, delta| {
                let delay = index as f32 * 0.14;
                let local = ((delta - delay) / (1.0 - delay)).clamp(0.0, 1.0);
                let diameter = 54.0 + local * 78.0;
                this.left(px(cx - diameter * 0.5))
                    .top(px(cy - diameter * 0.5))
                    .size(px(diameter))
                    .opacity((1.0 - local) * 0.75)
            },
        )
        .into_any_element()
}

fn complete_spark(gen: u64, index: usize, cx: f32, cy: f32) -> AnyElement {
    let angle = index as f32 * std::f32::consts::TAU / 8.0;
    let (sin, cos) = angle.sin_cos();
    div()
        .absolute()
        .size(px(6.))
        .rounded_full()
        .bg(rgb(if index.is_multiple_of(2) {
            theme::ACCENT
        } else {
            theme::OK
        }))
        .with_animation(
            SharedString::from(format!("intake-complete-spark-{gen}-{index}")),
            Animation::new(Duration::from_millis(720)),
            move |this, delta| {
                let delay = index as f32 * 0.035;
                let local = ((delta - delay) / (1.0 - delay)).clamp(0.0, 1.0);
                let radius = 22.0 + local * 42.0;
                this.left(px(cx + cos * radius - 3.))
                    .top(px(cy + sin * radius - 3.))
                    .opacity(if local < 0.3 {
                        local / 0.3
                    } else {
                        1.0 - (local - 0.3) / 0.7
                    })
            },
        )
        .into_any_element()
}

fn intake_copy(
    kind: IntakeKind,
    counts: IntakeCounts,
    batch: Option<&IntakePresentation>,
) -> (String, String) {
    if let Some(batch) = batch {
        if batch.counts.images == 0 {
            return (
                "No images".into(),
                "Only PNG, JPEG, and WebP are recognized.".into(),
            );
        }
        let (done, total) = batch.progress();
        if batch.phase != IntakePhase::Running {
            let (succeeded, failed) = batch.results();
            let sub = if failed == 0 {
                if total == 1 {
                    "1 image recognized".into()
                } else {
                    format!("{total} images recognized")
                }
            } else {
                format!("{succeeded} of {total} images recognized · {failed} failed")
            };
            return ("Complete".into(), sub);
        }
        let current = (done + 1).min(total);
        return ("Recognizing".into(), format!("{current} of {total}"));
    }
    let sub = count_line(counts);
    match kind {
        IntakeKind::Hover if counts.images == 0 => (
            "No images".into(),
            "Only PNG, JPEG, and WebP are recognized.".into(),
        ),
        IntakeKind::Hover => ("Drop to recognize".into(), sub),
        IntakeKind::Flash if counts.images == 0 => (
            "No images".into(),
            "Only PNG, JPEG, and WebP are recognized.".into(),
        ),
        IntakeKind::Flash => ("Recognizing".into(), sub),
    }
}

fn count_line(counts: IntakeCounts) -> String {
    if counts.images == 0 {
        "Only PNG, JPEG, and WebP are recognized.".into()
    } else if counts.images == 1 {
        "1 image will be recognized.".into()
    } else {
        format!("{} images will be recognized.", counts.images)
    }
}

fn paint_platen_dots(bounds: Bounds<Pixels>, window: &mut gpui::Window) {
    window.paint_quad(fill(bounds, rgb(theme::PAPER)));
    let ox: f32 = bounds.origin.x.into();
    let oy: f32 = bounds.origin.y.into();
    let right: f32 = ox + f32::from(bounds.size.width);
    let bottom: f32 = oy + f32::from(bounds.size.height);
    let mut x = ox + 10.0;
    while x < right {
        let mut y = oy + 10.0;
        while y < bottom {
            window.paint_quad(fill(
                Bounds::from_corners(point(px(x), px(y)), point(px(x + 2.0), px(y + 2.0))),
                rgb(theme::PAPER_DOT),
            ));
            y += DOT_PITCH;
        }
        x += DOT_PITCH;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn slot(index: usize) -> IntakeSlot {
        IntakeSlot {
            key: index.to_string().into(),
            work: IntakeWork::Working,
            visual: IntakeVisual::Visible,
            thumb: None,
            slot: index,
            degrees: intake_paper_spec(1, index as u64 + 1, index).degrees,
        }
    }

    #[test]
    fn check_anchor_rotates_with_the_paper() {
        let neutral = rotated_check_offset(0.0);
        assert_eq!(neutral, (35.0, -35.0));
        let clockwise = rotated_check_offset(9.0);
        assert!(clockwise.0 > neutral.0);
        assert!(clockwise.1 > neutral.1);
        let counter_clockwise = rotated_check_offset(-9.0);
        assert!(counter_clockwise.0 < neutral.0);
        assert!(counter_clockwise.1 < neutral.1);
    }

    #[test]
    fn few_images_are_centered_without_reassigning_slots() {
        let slots = vec![slot(0), slot(1), slot(2)];
        assert_eq!(reel_center_offset(&slots, 0), 99.0);
        assert_eq!(intake_pose(0).dx + 99.0, -66.0);
        assert_eq!(intake_pose(2).dx + 99.0, 66.0);
    }

    #[test]
    fn tail_images_center_as_a_group_and_keep_gaps() {
        let slots = vec![slot(1), slot(4)];
        let offset = reel_center_offset(&slots, 0);
        assert_eq!(offset, 0.0);
        assert_eq!(intake_pose(1).dx + offset, -99.0);
        assert_eq!(intake_pose(4).dx + offset, 99.0);
    }

    #[test]
    fn backlog_keeps_the_six_position_layout() {
        let slots = vec![slot(0), slot(1), slot(2), slot(3), slot(4)];
        assert_eq!(reel_center_offset(&slots, 1), 0.0);
    }

    #[test]
    fn completed_card_leaves_in_place_and_refills_its_slot() {
        let paths = (0..7)
            .map(|index| PathBuf::from(format!("{index}.png")))
            .collect();
        let mut batch = IntakeBatch::from_paths(paths, 0, 4);
        let (mut presentation, feedback) = IntakePresentation::new(&batch);
        assert!(feedback.is_empty());
        assert_eq!(presentation.backlog_count(), 2);

        assert!(batch.finish_key(2, true));
        assert_eq!(presentation.sync(&batch), vec![2]);
        assert!(presentation.begin_leave(2));
        assert_eq!(presentation.mark_gone_and_promote(2), None);
        let replacement = presentation
            .cards
            .iter()
            .find(|card| card.key == 6)
            .expect("first backlog card promoted");
        assert_eq!(replacement.slot, Some(1));
        assert_eq!(replacement.visual, IntakeVisual::Visible);
    }

    #[test]
    fn terminal_backlog_card_shows_feedback_when_promoted() {
        let paths = (0..6)
            .map(|index| PathBuf::from(format!("{index}.png")))
            .collect();
        let mut batch = IntakeBatch::from_paths(paths, 0, 5);
        let (mut presentation, _) = IntakePresentation::new(&batch);
        assert!(batch.finish_key(6, true));
        assert!(batch.finish_key(1, true));
        assert_eq!(presentation.sync(&batch), vec![1]);
        assert!(presentation.begin_leave(1));
        assert_eq!(presentation.mark_gone_and_promote(1), Some(6));
    }

    #[test]
    fn complete_copy_reports_successes_and_failures_separately() {
        let mut batch =
            IntakeBatch::from_paths(vec![PathBuf::from("a.png"), PathBuf::from("b.png")], 0, 9);
        assert!(batch.finish_key(1, true));
        assert!(batch.finish_key(2, false));
        let (mut presentation, _) = IntakePresentation::new(&batch);
        presentation.phase = IntakePhase::Complete;
        assert_eq!(
            intake_copy(IntakeKind::Flash, presentation.counts, Some(&presentation)),
            (
                "Complete".into(),
                "1 of 2 images recognized · 1 failed".into()
            )
        );
    }

    #[test]
    fn mixed_hover_copy_omits_unsupported_count() {
        assert_eq!(
            count_line(IntakeCounts {
                images: 3,
                skipped: 4,
            }),
            "3 images will be recognized."
        );
    }
}
