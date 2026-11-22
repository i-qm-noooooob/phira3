prpr_l10n::tl_file!("settings");

use super::{request_export, resolve_export, take_export, ExportConfig, Page, SharedState};
use crate::replay::library::{self, ReplayItem};
use anyhow::Result;
use inputbox::InputBox;
use macroquad::prelude::*;
use prpr::{
    ext::{poll_future, semi_black, LocalTask, RectExt},
    scene::{request_file, request_input, show_error, take_input, NextScene},
    ui::{button_hit, DRectButton, Dialog, RectButton, Ui},
};
use std::{
    borrow::Cow,
    io::{BufWriter, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
};

static DIRTY: AtomicBool = AtomicBool::new(false);

fn replay_matches_search(item: &ReplayItem, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || [
            item.chart_name.as_deref().unwrap_or_default(),
            item.player.as_deref().unwrap_or_default(),
            item.file.as_str(),
        ]
        .iter()
        .any(|value| value.to_lowercase().contains(&query))
}

pub fn mark_dirty() {
    DIRTY.store(true, Ordering::Release);
}

pub struct ReplayLibraryPage {
    items: Vec<ReplayItem>,
    item_buttons: Vec<DRectButton>,
    visible_items: usize,
    visible_start: usize,
    page_size: usize,
    prev_page_btn: DRectButton,
    next_page_btn: DRectButton,
    selected: Option<usize>,
    import_btn: DRectButton,
    refresh_btn: DRectButton,
    search_btn: DRectButton,
    search_clear_btn: RectButton,
    search: String,
    sort_btn: DRectButton,
    sort_mode: usize,
    keep_btn: DRectButton,
    delete_btn: DRectButton,
    export_btn: DRectButton,
    match_btn: DRectButton,
    play_btn: DRectButton,
    match_task: LocalTask<Result<(String, library::Match)>>,
    play_task: LocalTask<Result<crate::scene::ReplayScene>>,
    ready_scene: Option<crate::scene::ReplayScene>,
    status: Option<String>,
    delete_confirm: Arc<AtomicBool>,
    delete_file: Option<String>,
    export_file: Option<String>,
    export_task: Option<Receiver<Result<()>>>,
    error: Option<String>,
}

impl ReplayLibraryPage {
    pub fn new() -> Self {
        let mut page = Self {
            items: Vec::new(),
            item_buttons: Vec::new(),
            visible_items: 0,
            visible_start: 0,
            page_size: 1,
            prev_page_btn: DRectButton::new(),
            next_page_btn: DRectButton::new(),
            selected: None,
            import_btn: DRectButton::new(),
            refresh_btn: DRectButton::new(),
            search_btn: DRectButton::new(),
            search_clear_btn: RectButton::new(),
            search: String::new(),
            sort_btn: DRectButton::new(),
            sort_mode: 0,
            keep_btn: DRectButton::new(),
            delete_btn: DRectButton::new(),
            export_btn: DRectButton::new(),
            match_btn: DRectButton::new(),
            play_btn: DRectButton::new(),
            match_task: None,
            play_task: None,
            ready_scene: None,
            status: None,
            delete_confirm: Arc::default(),
            delete_file: None,
            export_file: None,
            export_task: None,
            error: None,
        };
        page.reload();
        page
    }

    fn reload(&mut self) {
        match library::list() {
            Ok(items) => {
                self.items = items;
                self.item_buttons.resize_with(self.items.len(), DRectButton::new);
                self.selected = self.selected.filter(|index| *index < self.items.len());
                self.visible_start = self.visible_start.min(self.filtered_indices().len().saturating_sub(1));
                self.error = None;
            }
            Err(err) => self.error = Some(err.to_string()),
        }
    }

    fn filtered_indices(&self) -> Vec<usize> {
        let mut indices = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| replay_matches_search(item, &self.search).then_some(index))
            .collect::<Vec<_>>();
        match self.sort_mode {
            1 => indices.sort_by(|a, b| {
                self.items[*a]
                    .chart_name
                    .as_deref()
                    .unwrap_or_default()
                    .to_lowercase()
                    .cmp(&self.items[*b].chart_name.as_deref().unwrap_or_default().to_lowercase())
                    .then_with(|| self.items[*a].file.cmp(&self.items[*b].file))
            }),
            2 => indices.sort_by(|a, b| self.items[*b].size.cmp(&self.items[*a].size)),
            _ => indices.sort_by(|a, b| {
                self.items[*b]
                    .recorded_at
                    .cmp(&self.items[*a].recorded_at)
                    .then_with(|| self.items[*b].imported_at.cmp(&self.items[*a].imported_at))
                    .then_with(|| self.items[*a].file.cmp(&self.items[*b].file))
            }),
        }
        indices
    }

    fn selected_item(&self) -> Option<&ReplayItem> {
        self.selected.and_then(|index| self.items.get(index))
    }

    fn start_export(&mut self, file: String) {
        let name = file.trim_end_matches(".phirarec").to_owned() + ".phirarec";
        if request_export(name) {
            self.export_file = Some(file);
        }
    }
}

impl Page for ReplayLibraryPage {
    fn label(&self) -> Cow<'static, str> {
        tl!("replay-library-title")
    }

    fn update(&mut self, _s: &mut SharedState) -> Result<()> {
        if let Some((id, value)) = take_input() {
            if id == "replay-search" {
                self.search = value;
                self.visible_start = 0;
                self.selected = None;
                self.status = None;
            }
        }
        if DIRTY.swap(false, Ordering::AcqRel) {
            self.reload();
        }

        if self.delete_confirm.swap(false, Ordering::AcqRel) {
            if let Some(file) = self.delete_file.take() {
                match library::delete(&file) {
                    Ok(()) => self.reload(),
                    Err(err) => show_error(err),
                }
            }
        }

        if let Some(result) = self.match_task.as_mut().and_then(|task| poll_future(task.as_mut())) {
            self.match_task = None;
            match result {
                Ok((file, library::Match::Found(local, verified))) => match library::bind(&file, &local, verified) {
                    Ok(()) => {
                        self.status = Some(if verified {
                            tl!("replay-library-match-verified").into_owned()
                        } else {
                            tl!("replay-library-match-unverified").into_owned()
                        });
                        self.reload();
                    }
                    Err(err) => show_error(err),
                },
                Ok((_, library::Match::Missing)) => self.status = Some(tl!("replay-library-match-missing").into_owned()),
                Ok((_, library::Match::WrongVersion)) => self.status = Some(tl!("replay-library-match-version").into_owned()),
                Ok((_, library::Match::WrongAudio)) => self.status = Some(tl!("replay-library-match-audio").into_owned()),
                Ok((_, library::Match::WrongServer)) => self.status = Some(tl!("replay-library-match-server").into_owned()),
                Err(err) => {
                    self.status = Some(tl!("replay-library-match-failed").into_owned());
                    show_error(err);
                }
            }
        }

        if let Some(result) = self.play_task.as_mut().and_then(|task| poll_future(task.as_mut())) {
            self.play_task = None;
            match result {
                Ok(scene) => self.ready_scene = Some(scene),
                Err(err) => show_error(err),
            }
        }

        if self.export_file.is_some() && self.export_task.is_none() {
            if let Some(config) = take_export() {
                let file = self.export_file.take().unwrap();
                match config {
                    Err(err) => show_error(err.into()),
                    Ok(ExportConfig { file: output, deleter }) => {
                        let (tx, rx) = mpsc::channel();
                        self.export_task = Some(rx);
                        std::thread::spawn(move || {
                            let result = (|| -> Result<()> {
                                let mut writer = BufWriter::new(output);
                                library::export_to_writer(&file, &mut writer)?;
                                writer.flush()?;
                                Ok(())
                            })();
                            if result.is_err() {
                                let _ = deleter();
                            }
                            let _ = tx.send(result);
                        });
                    }
                }
            }
        }

        if let Some(rx) = &self.export_task {
            match rx.try_recv() {
                Ok(Ok(())) => {
                    resolve_export();
                    self.export_task = None;
                }
                Ok(Err(err)) => {
                    show_error(err);
                    self.export_task = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => {
                    show_error(anyhow::anyhow!("Replay export thread panicked"));
                    self.export_task = None;
                }
            }
        }
        Ok(())
    }

    fn touch(&mut self, touch: &Touch, s: &mut SharedState) -> Result<bool> {
        let t = s.t;
        if self.prev_page_btn.touch(touch, t) {
            self.visible_start = self.visible_start.saturating_sub(self.page_size);
            return Ok(true);
        }
        if self.next_page_btn.touch(touch, t) {
            self.visible_start = (self.visible_start + self.page_size).min(self.items.len().saturating_sub(1));
            return Ok(true);
        }
        if !self.search.is_empty() && self.search_clear_btn.touch(touch) {
            button_hit();
            self.search.clear();
            self.visible_start = 0;
            self.selected = None;
            self.status = None;
            return Ok(true);
        }
        if (self.search.is_empty() || !self.search_clear_btn.contains(touch.position)) && self.search_btn.touch(touch, t) {
            request_input("replay-search", InputBox::new().default_text(&self.search));
            return Ok(true);
        }
        if self.sort_btn.touch(touch, t) {
            self.sort_mode = (self.sort_mode + 1) % 3;
            self.visible_start = 0;
            return Ok(true);
        }
        if self.import_btn.touch(touch, t) {
            request_file("_import_replay");
            return Ok(true);
        }
        if self.refresh_btn.touch(touch, t) {
            self.reload();
            return Ok(true);
        }

        if self.keep_btn.touch(touch, t) {
            if let Some((file, keep)) = self.selected_item().map(|item| (item.file.clone(), item.keep)) {
                if let Err(err) = library::set_keep(&file, !keep) {
                    show_error(err);
                } else {
                    self.reload();
                }
            }
            return Ok(true);
        }
        if self.delete_btn.touch(touch, t) {
            if let Some(item) = self.selected_item() {
                let file = item.file.clone();
                self.delete_file = Some(file);
                Dialog::plain(tl!("replay-library-delete-title"), tl!("replay-library-delete-confirm"))
                    .buttons(vec![tl!("cancel").into_owned(), tl!("confirm").into_owned()])
                    .listener({
                        let confirm = Arc::clone(&self.delete_confirm);
                        move |_dialog, id| {
                            if id == 1 {
                                confirm.store(true, Ordering::Release);
                            }
                            false
                        }
                    })
                    .show();
            }
            return Ok(true);
        }
        if self.export_btn.touch(touch, t) {
            if let Some(file) = self.selected_item().map(|item| item.file.clone()) {
                self.start_export(file);
            }
            return Ok(true);
        }
        if self.match_btn.touch(touch, t) {
            if self.match_task.is_none() {
                if let Some((file, preferred)) = self.selected_item().map(|item| (item.file.clone(), item.local_path.clone())) {
                    let candidates = crate::get_data().charts.iter().map(|chart| chart.local_path.clone()).collect::<Vec<_>>();
                    self.status = Some(tl!("replay-library-matching").into_owned());
                    self.match_task = Some(Box::pin(async move {
                        let replay = library::load(&file)?;
                        let result = library::match_chart(&replay, preferred.as_deref(), &candidates).await?;
                        Ok((file, result))
                    }));
                }
            }
            return Ok(true);
        }

        if self.play_btn.touch(touch, t) {
            if self.play_task.is_none() && self.ready_scene.is_none() {
                if let Some(file) = self.selected_item().map(|item| item.file.clone()) {
                    self.status = Some(tl!("replay-library-loading").into_owned());
                    self.play_task = Some(Box::pin(async move { crate::scene::ReplayScene::new(file).await }));
                }
            }
            return Ok(true);
        }

        let filtered = self.filtered_indices();
        for (slot, button) in self.item_buttons.iter_mut().take(self.visible_items).enumerate() {
            if button.touch(touch, t) {
                self.selected = filtered.get(self.visible_start + slot).copied();
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn next_scene(&mut self, _s: &mut SharedState) -> NextScene {
        self.ready_scene
            .take()
            .map(|scene| NextScene::Overlay(Box::new(scene)))
            .unwrap_or_default()
    }

    fn render(&mut self, ui: &mut Ui, s: &mut SharedState) -> Result<()> {
        let t = s.t;
        s.render_fader(ui, |ui| {
            let area = ui.content_rect().feather(-0.025);
            ui.fill_path(&area.rounded(0.02), semi_black(0.48));
            ui.text(tl!("replay-library-title")).pos(area.x + 0.025, area.y + 0.025).size(0.55).draw();

            let button_y = area.y + 0.105;
            let button_w = (area.w - 0.08) / 2.;
            self.import_btn
                .render_text(ui, Rect::new(area.x + 0.025, button_y, button_w, 0.075), t, tl!("replay-library-import"), 0.34, false);
            self.refresh_btn.render_text(
                ui,
                Rect::new(area.x + 0.055 + button_w, button_y, button_w, 0.075),
                t,
                tl!("replay-library-refresh"),
                0.34,
                false,
            );

            let search_y = button_y + 0.09;
            let search_w = (area.w - 0.075) * 0.70;
            let search_rect = Rect::new(area.x + 0.025, search_y, search_w, 0.065);
            self.search_btn.render_text(
                ui,
                search_rect,
                t,
                if self.search.is_empty() { tl!("replay-library-search") } else { Cow::Borrowed(self.search.as_str()) },
                0.28,
                false,
            );
            if !self.search.is_empty() {
                let clear_rect = Rect::new(search_rect.right() - search_rect.h, search_rect.y, search_rect.h, search_rect.h);
                ui.text("×").pos(clear_rect.center().x, clear_rect.center().y).anchor(0.5, 0.5).size(0.35).draw();
                self.search_clear_btn.set(ui, clear_rect);
            }
            let sort_rect = Rect::new(search_rect.right() + 0.025, search_y, area.w - search_w - 0.10, 0.065);
            let sort_label = match self.sort_mode {
                1 => tl!("replay-library-sort-name"),
                2 => tl!("replay-library-sort-size"),
                _ => tl!("replay-library-sort-recent"),
            };
            self.sort_btn.render_text(ui, sort_rect, t, sort_label, 0.25, false);
            let action_y = search_y + 0.075;
            let action_w = (area.w - 0.15) / 5.;
            let (has_selected, keep_label) = self
                .selected_item()
                .map(|item| {
                    (
                        true,
                        if item.keep {
                            tl!("replay-library-unkeep")
                        } else {
                            tl!("replay-library-keep")
                        },
                    )
                })
                .unwrap_or_else(|| (false, tl!("replay-library-select")));
            self.keep_btn
                .render_text(ui, Rect::new(area.x + 0.025, action_y, action_w, 0.07), t, keep_label, 0.28, has_selected);
            self.delete_btn.render_text(
                ui,
                Rect::new(area.x + 0.05 + action_w, action_y, action_w, 0.07),
                t,
                tl!("replay-library-delete"),
                0.28,
                has_selected,
            );
            self.export_btn.render_text(
                ui,
                Rect::new(area.x + 0.075 + action_w * 2., action_y, action_w, 0.07),
                t,
                tl!("replay-library-export"),
                0.28,
                has_selected,
            );
            self.match_btn.render_text(
                ui,
                Rect::new(area.x + 0.10 + action_w * 3., action_y, action_w, 0.07),
                t,
                tl!("replay-library-match"),
                0.28,
                has_selected && self.match_task.is_none(),
            );
            self.play_btn.render_text(
                ui,
                Rect::new(area.x + 0.125 + action_w * 4., action_y, action_w, 0.07),
                t,
                tl!("replay-library-play"),
                0.28,
                has_selected && self.play_task.is_none() && self.ready_scene.is_none(),
            );

            let mut y = action_y + 0.095;
            if let Some(status) = &self.status {
                ui.text(status).pos(area.x + 0.025, y).size(0.26).max_width(area.w - 0.05).draw();
                y += 0.045;
            }
            if let Some(error) = &self.error {
                ui.text(format!("{}: {error}", tl!("replay-library-error")))
                    .pos(area.x + 0.025, y)
                    .size(0.3)
                    .max_width(area.w - 0.05)
                    .draw();
                y += 0.08;
            }
            let filtered = self.filtered_indices();
            ui.text(format!("{} / {} {}", filtered.len(), self.items.len(), tl!("replay-library-count")))
                .pos(area.x + 0.025, y)
                .size(0.32)
                .draw();
            y += 0.055;

            if filtered.is_empty() {
                ui.text(if self.items.is_empty() {
                    tl!("replay-library-empty")
                } else {
                    tl!("replay-library-no-matches")
                })
                .pos(area.x + 0.025, y)
                .size(0.31)
                .max_width(area.w - 0.05)
                .draw();
            } else {
                let line_h = 0.09;
                let available = ((area.bottom() - y - 0.10) / line_h).floor().max(0.) as usize;
                self.page_size = available.max(1);
                self.visible_start = self.visible_start.min(filtered.len().saturating_sub(1));
                self.visible_items = self.page_size.min(filtered.len().saturating_sub(self.visible_start));
                for index in filtered.iter().copied().skip(self.visible_start).take(self.visible_items) {
                    let item = &self.items[index];
                    let rect = Rect::new(area.x + 0.015, y, area.w - 0.03, line_h - 0.008);
                    self.item_buttons[index].render_text(
                        ui,
                        rect,
                        t,
                        format!(
                            "{} — {}\nv{} · {} bytes · {}",
                            item.chart_name.as_deref().unwrap_or(tl!("replay-library-untitled").as_ref()),
                            item.player.as_deref().unwrap_or(tl!("replay-library-unknown-player").as_ref()),
                            item.version,
                            item.size,
                            if item.keep {
                                tl!("replay-library-kept")
                            } else {
                                tl!("replay-library-local")
                            }
                        ),
                        0.26,
                        self.selected == Some(index),
                    );
                    y += line_h;
                }
                if filtered.len() > self.visible_items {
                    let page = self.visible_start / self.page_size + 1;
                    let pages = filtered.len().div_ceil(self.page_size);
                    ui.text(format!("{page}/{pages}"))
                        .pos(area.x + area.w * 0.45, area.bottom() - 0.045)
                        .size(0.25)
                        .draw();
                    self.prev_page_btn.render_text(
                        ui,
                        Rect::new(area.x + 0.025, area.bottom() - 0.07, 0.16, 0.055),
                        t,
                        "‹",
                        0.28,
                        self.visible_start > 0,
                    );
                    self.next_page_btn.render_text(
                        ui,
                        Rect::new(area.right() - 0.185, area.bottom() - 0.07, 0.16, 0.055),
                        t,
                        "›",
                        0.28,
                        self.visible_start + self.visible_items < filtered.len(),
                    );
                }
            }
        });
        Ok(())
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> ReplayItem {
        ReplayItem {
            file: "replay-abc123.phirarec".to_owned(),
            version: 6,
            size: 1024,
            imported_at: 0,
            keep: false,
            local_path: None,
            verified: false,
            chart_name: Some("Freedom Dive".to_owned()),
            player: Some("PlayerOne".to_owned()),
            recorded_at: None,
        }
    }

    #[test]
    fn replay_search_matches_chart_player_and_filename_case_insensitively() {
        let item = item();
        assert!(replay_matches_search(&item, "freedom"));
        assert!(replay_matches_search(&item, "PLAYERONE"));
        assert!(replay_matches_search(&item, "ABC123"));
        assert!(replay_matches_search(&item, "  dive  "));
        assert!(replay_matches_search(&item, ""));
        assert!(!replay_matches_search(&item, "unrelated"));
    }
}
