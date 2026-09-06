use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, Image, Priority, RenderImage, Window, prelude::*};
use uuid::Uuid;

use super::intake_paper::IntakePaperSpec;
use super::main_window::MainWindow;
use crate::cache::{MediaCache, thumb_retain_ids};
use crate::preview::{
    DocDerived, derived_copy_rows, document_preview_with_dpr, raster_dpr, should_spawn_derived,
};

pub(crate) struct WindowMedia {
    pub(crate) cache: Rc<RefCell<MediaCache>>,
    pub(crate) derived: Option<DocDerived>,
    delayed_error: Option<DocDerived>,
    derived_busy: bool,
    gc_scheduled: bool,
    pub(crate) last_scale: f32,
    pub(crate) thumb_keep: Rc<RefCell<Vec<Uuid>>>,
    thumb_inflight: Rc<RefCell<HashSet<Uuid>>>,
    full_inflight: HashSet<Uuid>,
    intake_polaroids: Rc<RefCell<HashMap<Uuid, IntakePaperRender>>>,
    intake_polaroid_inflight: Rc<RefCell<HashSet<Uuid>>>,
    intake_polaroid_keep: Rc<RefCell<HashSet<Uuid>>>,
}

struct IntakePaperRender {
    image: Arc<RenderImage>,
}

impl WindowMedia {
    pub(crate) fn new(scale: f32) -> Self {
        Self {
            cache: Rc::new(RefCell::new(MediaCache::new())),
            derived: None,
            delayed_error: None,
            derived_busy: false,
            gc_scheduled: false,
            last_scale: scale,
            thumb_keep: Rc::new(RefCell::new(Vec::new())),
            thumb_inflight: Rc::new(RefCell::new(HashSet::new())),
            full_inflight: HashSet::new(),
            intake_polaroids: Rc::new(RefCell::new(HashMap::new())),
            intake_polaroid_inflight: Rc::new(RefCell::new(HashSet::new())),
            intake_polaroid_keep: Rc::new(RefCell::new(HashSet::new())),
        }
    }

    fn intake_polaroid(&self, id: Uuid) -> Option<Arc<RenderImage>> {
        self.intake_polaroids
            .borrow()
            .get(&id)
            .map(|paper| paper.image.clone())
    }
}

impl MainWindow {
    pub(crate) fn full(&self, id: Uuid) -> Option<Arc<RenderImage>> {
        self.media.cache.borrow().full(id)
    }

    pub(crate) fn math_image(&self, svg: &str, cx: &mut App) -> Arc<Image> {
        self.media.cache.borrow_mut().math_image(svg, cx)
    }

    pub(crate) fn schedule_media_gc(&mut self, cx: &mut Context<Self>) {
        if self.media.gc_scheduled {
            return;
        }
        self.media.gc_scheduled = true;
        let entity = cx.entity();
        cx.defer(move |cx| {
            entity.update(cx, |this, cx| {
                this.media.gc_scheduled = false;
                let (keep_thumbs, keep_fulls, pin) = {
                    let state = this.state.read(cx);
                    let keep_fulls = state.gpu_full_ids();
                    let pin = state.selected();
                    let kept = this.media.thumb_keep.borrow().clone();
                    let keep_thumbs = thumb_retain_ids(this.orig.open, &kept, state.visible_ids());
                    (keep_thumbs, keep_fulls, pin)
                };
                let mut media = this.media.cache.borrow_mut();
                media.retain_thumbs(keep_thumbs.into_iter(), cx);
                media.retain_fulls(keep_fulls.into_iter(), pin, cx);
                media.trim_math(cx);
            });
        });
    }

    pub(crate) fn release_hidden_media(&mut self, cx: &mut Context<Self>) {
        self.media.full_inflight.clear();
        self.media.thumb_inflight.borrow_mut().clear();
        self.media.derived = None;
        self.media.delayed_error = None;
        self.release_intake_polaroids(cx);
        self.media.cache.borrow_mut().clear(cx);
        cx.notify();
    }

    pub(crate) fn ensure_thumbs(&self, ids: &[Uuid], cx: &mut Context<Self>) {
        let mut request = Vec::new();
        let mut jobs = Vec::new();
        {
            let state = self.state.read(cx);
            let media = self.media.cache.borrow();
            let mut inflight = self.media.thumb_inflight.borrow_mut();
            for &id in ids {
                if media.thumb(id).is_some() || inflight.contains(&id) {
                    continue;
                }
                if state.thumbnail_failed(id) {
                    continue;
                }
                let Some(doc) = state.doc(id) else {
                    continue;
                };
                if matches!(doc.image, crate::doc::ImageSlot::Missing) {
                    continue;
                }
                if !doc.thumb_jpeg.is_empty() {
                    inflight.insert(id);
                    jobs.push((id, doc.thumb_jpeg.clone(), None));
                } else if let Some(px) = doc.image.pixels() {
                    inflight.insert(id);
                    jobs.push((id, Vec::new(), Some(px.clone())));
                } else {
                    request.push(id);
                }
            }
        }
        if !request.is_empty() {
            let entity = cx.entity();
            cx.defer(move |cx| {
                entity.update(cx, |this, cx| {
                    for id in request {
                        this.state.update(cx, |s, cx| s.request_thumb(id, cx));
                    }
                });
            });
        }
        for (id, jpeg, pixels) in jobs {
            cx.spawn(async move |this, cx| {
                let render = cx
                    .background_spawn(async move {
                        crate::cache::MediaCache::decode_thumb(&jpeg, pixels.as_deref())
                    })
                    .await;
                if let Err(err) = this.update(cx, |this, cx| {
                    this.media.thumb_inflight.borrow_mut().remove(&id);
                    if let Some(render) = render {
                        this.media.cache.borrow_mut().put_thumb(id, render);
                        cx.notify();
                    } else {
                        this.state
                            .update(cx, |state, _| state.mark_thumbnail_failed(id));
                    }
                }) {
                    eprintln!("thumb decode: {err}");
                }
            })
            .detach();
        }
    }

    pub(crate) fn release_intake_polaroids(&mut self, cx: &mut App) {
        self.media.intake_polaroid_keep.borrow_mut().clear();
        for (_, paper) in self.media.intake_polaroids.borrow_mut().drain() {
            cx.drop_image(paper.image, None);
        }
    }

    pub(crate) fn ensure_intake_polaroids(
        &self,
        jobs: &[(Uuid, IntakePaperSpec)],
        cx: &mut Context<Self>,
    ) {
        let keep: HashSet<_> = jobs.iter().map(|(id, _)| *id).collect();
        *self.media.intake_polaroid_keep.borrow_mut() = keep.clone();
        let stale: Vec<_> = self
            .media
            .intake_polaroids
            .borrow()
            .keys()
            .filter(|id| !keep.contains(id))
            .copied()
            .collect();
        for id in stale {
            if let Some(paper) = self.media.intake_polaroids.borrow_mut().remove(&id) {
                cx.drop_image(paper.image, None);
            }
        }
        let available = 2usize.saturating_sub(self.media.intake_polaroid_inflight.borrow().len());
        let mut pending = Vec::with_capacity(available);
        {
            let state = self.state.read(cx);
            let ready = self.media.intake_polaroids.borrow();
            let mut inflight = self.media.intake_polaroid_inflight.borrow_mut();
            for &(id, spec) in jobs {
                if pending.len() == available {
                    break;
                }
                if ready.contains_key(&id) || inflight.contains(&id) {
                    continue;
                }
                let Some(doc) = state.doc(id) else {
                    continue;
                };
                let Some(pixels) = doc.image.pixels() else {
                    continue;
                };
                inflight.insert(id);
                pending.push((id, spec, pixels.clone()));
            }
        }
        for (id, spec, pixels) in pending {
            cx.spawn(async move |this, cx| {
                let paper = cx
                    .background_executor()
                    .spawn_with_priority(Priority::Low, async move {
                        let image = super::intake_paper::intake_paper_image(pixels.as_ref(), spec);
                        IntakePaperRender {
                            image: crate::imgutil::rgba_to_render(&image),
                        }
                    })
                    .await;
                if let Err(err) = this.update(cx, |this, cx| {
                    this.media.intake_polaroid_inflight.borrow_mut().remove(&id);
                    if this.media.intake_polaroid_keep.borrow().contains(&id) {
                        this.media.intake_polaroids.borrow_mut().insert(id, paper);
                    } else {
                        cx.drop_image(paper.image, None);
                    }
                    this.refresh_intake_media(cx);
                    cx.notify();
                }) {
                    eprintln!("intake polaroid: {err}");
                }
            })
            .detach();
        }
    }

    pub(crate) fn intake_polaroid(&self, id: Uuid) -> Option<Arc<RenderImage>> {
        self.media.intake_polaroid(id)
    }

    pub(crate) fn ensure_selected_full(&mut self, cx: &mut Context<Self>) {
        if self.intake.is_some() {
            return;
        }
        let (id, pixels) = {
            let state = self.state.read(cx);
            let Some(doc) = state.selected_doc() else {
                return;
            };
            let Some(pixels) = doc.image.pixels() else {
                return;
            };
            (doc.id, pixels.clone())
        };
        if self.media.cache.borrow().full(id).is_some() || !self.media.full_inflight.insert(id) {
            return;
        }
        cx.spawn(async move |this, cx| {
            let render = cx
                .background_spawn(async move { crate::imgutil::gpu_display_image(pixels.as_ref()) })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.media.full_inflight.remove(&id);
                if this.state.read(cx).doc(id).is_some() {
                    this.media.cache.borrow_mut().put_full(id, render);
                    this.schedule_media_gc(cx);
                    cx.notify();
                }
            }) {
                eprintln!("full image prepare: {err}");
            }
        })
        .detach();
    }

    pub(crate) fn schedule_derived(&mut self, window: &Window, cx: &mut Context<Self>) {
        self.media.last_scale = window.scale_factor();
        self.schedule_derived_from_app(cx);
    }

    pub(crate) fn schedule_derived_from_app(&mut self, cx: &mut Context<Self>) {
        if self.intake.is_some() {
            return;
        }
        let dpr = raster_dpr(self.media.last_scale);
        let selected = {
            let state = self.state.read(cx);
            state.selected_doc().map(|doc| {
                (
                    doc.id,
                    doc.revision,
                    doc.has_ready_blocks() && !doc.source_pending && doc.source_error.is_none(),
                    state.prefs.clone(),
                )
            })
        };
        let Some((id, revision, ready, prefs)) = selected else {
            self.media.derived = None;
            self.media.delayed_error = None;
            return;
        };
        if self.media.derived.as_ref().is_some_and(|d| d.id != id) {
            self.media.derived = None;
        }
        if let Some(delayed) = self.media.delayed_error.take()
            && ready
            && delayed.matches(id, revision, dpr, &prefs)
        {
            if self
                .state
                .read(cx)
                .doc(id)
                .is_some_and(|doc| doc.source_feedback_ready())
            {
                self.media.derived = Some(delayed);
            } else {
                self.media.delayed_error = Some(delayed);
                return;
            }
        }
        if !ready {
            return;
        }
        if !should_spawn_derived(
            ready,
            self.media
                .derived
                .as_ref()
                .is_some_and(|d| d.matches(id, revision, dpr, &prefs)),
            self.media.derived_busy,
        ) {
            return;
        }
        let Some(blocks) = self.state.read(cx).doc(id).map(|d| d.blocks.clone()) else {
            return;
        };
        self.media.derived_busy = true;
        cx.spawn(async move |this, cx| {
            let built = cx
                .background_spawn(async move {
                    let preview = document_preview_with_dpr(&blocks, dpr, prefs.content_font);
                    let rows = derived_copy_rows(&blocks, &prefs);
                    DocDerived {
                        id,
                        revision,
                        dpr,
                        inline_delim: prefs.inline_delim,
                        block_delim: prefs.block_delim,
                        content_font: prefs.content_font,
                        preview: preview.into(),
                        copy_rows: rows,
                    }
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                this.media.derived_busy = false;
                let state = this.state.read(cx);
                if state.selected_doc().is_some_and(|doc| {
                    !doc.source_pending
                        && built.matches(
                            doc.id,
                            doc.revision,
                            raster_dpr(this.media.last_scale),
                            &state.prefs,
                        )
                }) {
                    if built
                        .preview
                        .iter()
                        .any(crate::preview::PreviewBlock::has_error)
                        && state
                            .selected_doc()
                            .is_some_and(|doc| !doc.source_feedback_ready())
                    {
                        this.media.delayed_error = Some(built);
                    } else {
                        this.media.derived = Some(built);
                    }
                }
                this.schedule_derived_from_app(cx);
                cx.notify();
            }) {
                eprintln!("{}: derived preview: {err}", crate::identity::APP_SLUG);
            }
        })
        .detach();
    }
}
