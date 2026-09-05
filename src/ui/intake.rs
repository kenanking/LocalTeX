use std::sync::Arc;
use std::time::Duration;

use gpui::{
    canvas, div, ease_out_quint, fill, img, point, prelude::*, px, relative, rgb, svg, Animation,
    AnimationExt, AnyElement, Bounds, Pixels, RenderImage, SharedString,
};

use super::theme;
use crate::state::{IntakeBatch, IntakeCounts, IntakeWork};

use super::intake_paper::{intake_paper_spec, IntakePaperSpec};

const PLATEN_W: f32 = 500.0;
const PLATEN_H: f32 = 330.0;
const DOT_PITCH: f32 = 20.0;
const REEL_W: f32 = 416.0;
const REEL_H: f32 = 118.0;
const CARD_SIZE: f32 = 82.0;
const PROGRESS_W: f32 = 330.0;
pub(crate) const INTAKE_REJECT_HOLD: Duration = Duration::from_millis(700);
const INTAKE_SUCCESS_FEEDBACK: Duration = Duration::from_millis(520);
const INTAKE_FAILURE_FEEDBACK: Duration = Duration::from_millis(160);
pub(crate) const INTAKE_COMPLETE_EFFECT: Duration = Duration::from_millis(360);
pub(crate) const INTAKE_COMPLETE_HOLD: Duration = Duration::from_millis(700);
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
    pub generation: u64,
    pub counts: IntakeCounts,
    pub phase: IntakePhase,
    cards: Vec<IntakeCard>,
    done: usize,
    succeeded: usize,
    failed: usize,
    slots: [Option<usize>; INTAKE_WINDOW],
    backlog: usize,
    next_backlog: usize,
}

#[derive(Default)]
pub(crate) struct IntakeSync {
    pub feedback: Vec<IntakeFeedback>,
    pub complete: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct IntakeFeedback {
    pub key: u64,
    pub succeeded: bool,
}

pub(crate) fn intake_feedback_duration(succeeded: bool) -> Duration {
    if succeeded {
        INTAKE_SUCCESS_FEEDBACK
    } else {
        INTAKE_FAILURE_FEEDBACK
    }
}

impl IntakePresentation {
    pub fn new(batch: &IntakeBatch) -> (Self, IntakeSync) {
        let mut presentation = Self {
            generation: batch.generation,
            counts: batch.counts(),
            phase: IntakePhase::Running,
            cards: Vec::new(),
            done: 0,
            succeeded: 0,
            failed: 0,
            slots: [None; INTAKE_WINDOW],
            backlog: 0,
            next_backlog: 0,
        };
        let feedback = presentation.sync(batch);
        (presentation, feedback)
    }

    pub fn sync(&mut self, batch: &IntakeBatch) -> IntakeSync {
        debug_assert_eq!(self.generation, batch.generation);
        self.counts = batch.counts();
        self.done = 0;
        self.succeeded = 0;
        self.failed = 0;

        let mut feedback = Vec::new();
        for (index, item) in batch.items.iter().enumerate() {
            if let Some(card) = self.cards.get_mut(index) {
                debug_assert_eq!(card.key, item.key);
                let became_terminal = !card.work.is_terminal() && item.work.is_terminal();
                card.id = item.id;
                card.work = item.work;
                if became_terminal && card.visual == IntakeVisual::Visible {
                    card.visual = IntakeVisual::Feedback;
                    feedback.push(IntakeFeedback {
                        key: card.key,
                        succeeded: item.work == IntakeWork::Succeeded,
                    });
                } else if became_terminal && card.visual == IntakeVisual::Backlog {
                    card.visual = IntakeVisual::Gone;
                    self.backlog = self.backlog.saturating_sub(1);
                }
            } else {
                let slot = self.first_vacant_slot();
                let visual = match (slot, item.work.is_terminal()) {
                    (Some(_), true) => IntakeVisual::Feedback,
                    (Some(_), false) => IntakeVisual::Visible,
                    (None, true) => IntakeVisual::Gone,
                    (None, false) => IntakeVisual::Backlog,
                };
                self.cards.push(IntakeCard {
                    key: item.key,
                    id: item.id,
                    work: item.work,
                    visual,
                    slot,
                });
                let card_index = self.cards.len() - 1;
                if let Some(slot) = slot {
                    self.slots[slot] = Some(card_index);
                } else if visual == IntakeVisual::Backlog {
                    self.backlog += 1;
                }
                if visual == IntakeVisual::Feedback {
                    feedback.push(IntakeFeedback {
                        key: item.key,
                        succeeded: item.work == IntakeWork::Succeeded,
                    });
                }
            }

            self.done += usize::from(item.work.is_terminal());
            self.succeeded += usize::from(item.work == IntakeWork::Succeeded);
            self.failed += usize::from(item.work == IntakeWork::Failed);
        }
        let complete = batch.all_terminal()
            && self.phase == IntakePhase::Running
            && !self.has_active_feedback();
        IntakeSync { feedback, complete }
    }

    fn first_vacant_slot(&self) -> Option<usize> {
        self.slots.iter().position(Option::is_none)
    }

    pub fn finish_feedback_and_promote(&mut self, key: u64) -> bool {
        if self.phase != IntakePhase::Running {
            return false;
        }
        let Some(index) = key.checked_sub(1).map(|key| key as usize) else {
            return false;
        };
        let slot = {
            let Some(card) = self.cards.get_mut(index) else {
                return false;
            };
            if card.key != key || card.visual != IntakeVisual::Feedback {
                return false;
            }
            card.visual = IntakeVisual::Gone;
            let Some(slot) = card.slot.take() else {
                return false;
            };
            self.slots[slot] = None;
            slot
        };
        while self.next_backlog < self.cards.len() {
            let replacement_index = self.next_backlog;
            self.next_backlog += 1;
            let replacement = &mut self.cards[replacement_index];
            if replacement.visual != IntakeVisual::Backlog {
                continue;
            }
            self.backlog = self.backlog.saturating_sub(1);
            if replacement.work.is_terminal() {
                replacement.visual = IntakeVisual::Gone;
                continue;
            }
            replacement.slot = Some(slot);
            replacement.visual = IntakeVisual::Visible;
            self.slots[slot] = Some(replacement_index);
            break;
        }
        true
    }

    pub fn backlog_count(&self) -> usize {
        self.backlog
    }

    pub fn ready_to_complete(&self) -> bool {
        self.counts.images > 0 && self.done == self.counts.images && !self.has_active_feedback()
    }

    fn has_active_feedback(&self) -> bool {
        self.visible_cards()
            .any(|(_, card)| card.visual == IntakeVisual::Feedback)
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

    pub fn paper_jobs(&self) -> Vec<(uuid::Uuid, IntakePaperSpec)> {
        self.visible_cards()
            .filter_map(|(slot, card)| {
                card.id
                    .map(|id| (id, intake_paper_spec(self.generation, card.key, slot)))
            })
            .collect()
    }

    fn visible_cards(&self) -> impl Iterator<Item = (usize, &IntakeCard)> {
        self.slots
            .iter()
            .enumerate()
            .filter_map(|(slot, index)| index.map(|index| (slot, &self.cards[index])))
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
    thumbs: impl Fn(uuid::Uuid) -> Option<Arc<RenderImage>>,
) -> Vec<IntakeSlot> {
    batch
        .visible_cards()
        .map(|(slot, item)| {
            let paper = intake_paper_spec(batch.generation, item.key, slot);
            IntakeSlot {
                key: SharedString::from(item.key.to_string()),
                work: item.work,
                visual: item.visual,
                thumb: item.id.and_then(&thumbs),
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
) -> impl IntoElement + use<> {
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
    let animation_key = batch.map_or_else(
        || SharedString::from("intake-platen-hover"),
        |batch| {
            SharedString::from(format!(
                "intake-platen-{}-{:?}",
                batch.generation, batch.phase
            ))
        },
    );
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
            animation_key,
            Animation::new(if fading {
                INTAKE_PLATEN_OUT
            } else if complete {
                INTAKE_COMPLETE_EFFECT
            } else {
                Duration::from_millis(180)
            })
            .with_easing(ease_out_quint()),
            move |this, delta| {
                let opacity = if fading { 1.0 - delta } else { delta.min(1.0) };
                this.opacity(opacity)
            },
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
    let feedback = slot.visual == IntakeVisual::Feedback;
    let succeeded = slot.work == IntakeWork::Succeeded;
    let thumb = slot.thumb.clone();
    let left = REEL_W * 0.5 + pose.dx + center_offset - CARD_SIZE * 0.5;
    let top = REEL_H * 0.5 + pose.dy - CARD_SIZE * 0.5;
    let pane = render_thumb_pane(slot, thumb, working);
    let inner = if feedback {
        div()
            .size_full()
            .child(pane)
            .with_animation(
                SharedString::from(format!("intake-feedback-{}", slot.key)),
                Animation::new(intake_feedback_duration(succeeded)),
                move |this, delta| {
                    let exit = if succeeded {
                        ((delta - 0.72) / 0.28).clamp(0.0, 1.0)
                    } else {
                        delta
                    };
                    this.p(px(exit * 5.)).opacity(1.0 - exit)
                },
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
        .when(thumb.is_none(), |d| {
            d.child(img("icons/intake-paper.svg").size_full())
        })
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
            Animation::new(Duration::from_millis(1_050))
                .repeat_synced()
                .with_max_fps(30.0),
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
    if slot.work != IntakeWork::Succeeded || slot.visual != IntakeVisual::Feedback {
        return None;
    }
    let pose = intake_pose(slot.slot);
    let (check_dx, check_dy) = rotated_check_offset(slot.degrees);
    let left = REEL_W * 0.5 + pose.dx + center_offset + check_dx - 13.0;
    let top = REEL_H * 0.5 + pose.dy + check_dy - 13.0;
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
                SharedString::from(format!("intake-check-{}", slot.key)),
                Animation::new(INTAKE_SUCCESS_FEEDBACK),
                |this, delta| {
                    let opacity = if delta < 0.18 {
                        delta / 0.18
                    } else if delta < 0.72 {
                        1.0
                    } else {
                        1.0 - (delta - 0.72) / 0.28
                    };
                    this.opacity(opacity)
                },
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
    let generation = batch.generation;
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
        .child(complete_ring(generation, 0, CX, CY))
        .child(complete_ring(generation, 1, CX, CY));
    for index in 0..8 {
        effect = effect.child(complete_spark(generation, index, CX, CY));
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
                    SharedString::from(format!("intake-complete-mark-{generation}")),
                    Animation::new(INTAKE_COMPLETE_EFFECT).with_easing(ease_out_quint()),
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

fn complete_ring(generation: u64, index: usize, cx: f32, cy: f32) -> AnyElement {
    div()
        .absolute()
        .rounded_full()
        .border_1()
        .border_color(rgb(0x79c990))
        .with_animation(
            SharedString::from(format!("intake-complete-ring-{generation}-{index}")),
            Animation::new(INTAKE_COMPLETE_EFFECT),
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

fn complete_spark(generation: u64, index: usize, cx: f32, cy: f32) -> AnyElement {
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
            SharedString::from(format!("intake-complete-spark-{generation}-{index}")),
            Animation::new(INTAKE_COMPLETE_EFFECT),
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
        let (mut presentation, sync) = IntakePresentation::new(&batch);
        assert!(sync.feedback.is_empty());
        assert_eq!(presentation.backlog_count(), 2);

        assert!(batch.finish_key(2, true));
        assert_eq!(
            presentation.sync(&batch).feedback,
            vec![IntakeFeedback {
                key: 2,
                succeeded: true,
            }]
        );
        assert!(presentation.finish_feedback_and_promote(2));
        let replacement = presentation
            .cards
            .iter()
            .find(|card| card.key == 6)
            .expect("first backlog card promoted");
        assert_eq!(replacement.slot, Some(1));
        assert_eq!(replacement.visual, IntakeVisual::Visible);
    }

    #[test]
    fn terminal_backlog_card_is_not_replayed_when_a_slot_opens() {
        let paths = (0..6)
            .map(|index| PathBuf::from(format!("{index}.png")))
            .collect();
        let mut batch = IntakeBatch::from_paths(paths, 0, 5);
        let (mut presentation, _) = IntakePresentation::new(&batch);
        assert!(batch.finish_key(6, true));
        assert!(batch.finish_key(1, true));
        assert_eq!(
            presentation.sync(&batch).feedback,
            vec![IntakeFeedback {
                key: 1,
                succeeded: true,
            }]
        );
        assert!(presentation.finish_feedback_and_promote(1));
        let terminal = presentation
            .cards
            .iter()
            .find(|card| card.key == 6)
            .expect("terminal backlog card exists");
        assert_eq!(terminal.visual, IntakeVisual::Gone);
        assert_eq!(terminal.slot, None);
        assert_eq!(presentation.backlog_count(), 0);
    }

    #[test]
    fn large_batch_sync_keeps_only_five_render_slots() {
        let paths = (0..200)
            .map(|index| PathBuf::from(format!("{index}.png")))
            .collect();
        let mut batch = IntakeBatch::from_paths(paths, 0, 8);
        for key in 1..=5 {
            batch.bind_item(key, uuid::Uuid::from_u128(key as u128));
        }
        let (presentation, sync) = IntakePresentation::new(&batch);
        assert!(!sync.complete);
        assert_eq!(presentation.visible_cards().count(), INTAKE_WINDOW);
        assert_eq!(presentation.backlog_count(), 195);
        assert_eq!(presentation.paper_jobs().len(), INTAKE_WINDOW);
    }

    #[test]
    fn all_terminal_batch_waits_only_for_visible_feedback() {
        let paths = (0..36)
            .map(|index| PathBuf::from(format!("{index}.png")))
            .collect();
        let mut batch = IntakeBatch::from_paths(paths, 0, 8);
        for key in 1..=36 {
            assert!(batch.finish_key(key, true));
        }
        let (mut presentation, sync) = IntakePresentation::new(&batch);
        assert!(!sync.complete);
        assert_eq!(sync.feedback.len(), INTAKE_WINDOW);
        assert!(sync.feedback.iter().all(|feedback| feedback.succeeded));
        for (index, feedback) in sync.feedback.into_iter().enumerate() {
            assert!(presentation.finish_feedback_and_promote(feedback.key));
            assert_eq!(presentation.ready_to_complete(), index + 1 == INTAKE_WINDOW);
        }
        assert_eq!(presentation.backlog_count(), 0);
    }

    #[test]
    fn failure_feedback_is_short_and_still_gates_completion() {
        let mut batch = IntakeBatch::from_paths(vec![PathBuf::from("a.png")], 0, 8);
        assert!(batch.finish_key(1, false));
        let (mut presentation, sync) = IntakePresentation::new(&batch);
        assert_eq!(
            sync.feedback,
            vec![IntakeFeedback {
                key: 1,
                succeeded: false,
            }]
        );
        assert!(!sync.complete);
        assert_eq!(intake_feedback_duration(false), INTAKE_FAILURE_FEEDBACK);
        assert!(presentation.finish_feedback_and_promote(1));
        assert!(presentation.ready_to_complete());
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
