use std::time::Duration;

use gpui::{AppContext, Context, Timer};
use uuid::Uuid;

use crate::doc::{DocStatus, Document};
use crate::identity::APP_SLUG;
use crate::library::merge_visible;
use crate::store::{self, CivilDate};

use super::{AppState, DatePreset};

impl AppState {
    pub fn set_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.search.query == query {
            return;
        }
        self.search.query = query;
        self.schedule_filter(cx);
    }

    pub fn set_date_preset(&mut self, preset: DatePreset, cx: &mut Context<Self>) {
        if !self.library.set_date_preset(preset) {
            return;
        }
        self.schedule_filter(cx);
    }

    pub(super) fn schedule_filter(&mut self, cx: &mut Context<Self>) {
        let gen = self.search.bump();
        let query = self.search.query.clone();
        let range = self
            .library
            .date_preset()
            .to_range(CivilDate::today_local());
        let store = self.store.clone();
        let inflight: Vec<(Uuid, std::time::SystemTime, String)> = self
            .library
            .iter_all()
            .filter(|d| !d.persisted)
            .map(|d| {
                let blob = if matches!(d.status, DocStatus::Ready) {
                    Document::search_text_for_blocks(&d.blocks)
                } else {
                    d.first_line()
                };
                (d.id, d.created_at, blob)
            })
            .collect();
        let ram_only: Option<Vec<(Uuid, std::time::SystemTime, String)>> = if store.is_none() {
            Some(
                self.library
                    .iter_all()
                    .filter(|d| d.persisted)
                    .map(|d| {
                        (
                            d.id,
                            d.created_at,
                            Document::search_text_for_blocks(&d.blocks),
                        )
                    })
                    .collect(),
            )
        } else {
            None
        };
        cx.spawn(async move |this, cx| {
            Timer::after(Duration::from_millis(120)).await;
            let ids = cx
                .background_spawn(async move {
                    let persisted = if let Some(store) = store {
                        store.query_ids(&query, range).unwrap_or_else(|err| {
                            eprintln!("{APP_SLUG}: search: {err}");
                            Vec::new()
                        })
                    } else {
                        ram_only
                            .unwrap_or_default()
                            .into_iter()
                            .filter(|(_, created, blob)| {
                                store::instant_in_range(*created, range)
                                    && text_matches(&query, blob)
                            })
                            .map(|(id, _, _)| id)
                            .collect()
                    };
                    let inflight_hits: Vec<Uuid> = inflight
                        .into_iter()
                        .filter(|(_, created, blob)| {
                            store::instant_in_range(*created, range) && text_matches(&query, blob)
                        })
                        .map(|(id, _, _)| id)
                        .collect();
                    merge_visible(inflight_hits, persisted)
                })
                .await;
            if let Err(err) = this.update(cx, |this, cx| {
                if this.search.gen != gen {
                    return;
                }
                if this.library.set_visible(ids) {
                    if let Some(id) = this.library.selected() {
                        this.ensure_detail(id, cx);
                    }
                }
                cx.notify();
            }) {
                eprintln!("{APP_SLUG}: filter task: {err}");
            }
        })
        .detach();
    }
}

fn text_matches(query: &str, blob: &str) -> bool {
    let q = query.trim();
    q.is_empty() || blob.to_lowercase().contains(&q.to_lowercase())
}
