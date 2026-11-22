prpr_l10n::tl_file!("settings");

use super::{
    request_export, resolve_export, take_export, ExportConfig, Page, SharedState,
};
use crate::replay::library::{self, ReplayItem};
use anyhow::Result;
use macroquad::prelude::*;
use prpr::{
    ext::{semi_black, RectExt},
    scene::{request_file, show_error},
    ui::{DRectButton, Dialog, Ui},
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

pub fn mark_dirty() {
    DIRTY.store(true, Ordering::Release);
}

pub struct ReplayLibraryPage {
    items: Vec<ReplayItem>,
    item_buttons: Vec<DRectButton>,
    selected: Option<usize>,
    import_btn: DRectButton,
    refresh_btn: DRectButton,
    keep_btn: DRectButton,
    delete_btn: DRectButton,
    export_btn: DRectButton,
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
            selected: None,
            import_btn: DRectButton::new(),
            refresh_btn: DRectButton::new(),
            keep_btn: DRectButton::new(),
            delete_btn: DRectButton::new(),
            export_btn: DRectButton::new(),
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
                self.error = None;
            }
            Err(err) => self.error = Some(err.to_string()),
        }
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
        let area = s.content_rect().feather(-0.025);
        let button_y = area.y + 0.105;
        let button_w = (area.w - 0.08) / 2.;
        if self.import_btn.touch(touch, t) {
            request_file("_import_replay");
            return Ok(true);
        }
        if self.refresh_btn.touch(touch, t) {
            self.reload();
            return Ok(true);
        }

        let action_y = button_y + 0.09;
        let action_w = (area.w - 0.10) / 3.;
        if self.keep_btn.touch(touch, t) {
            if let Some(item) = self.selected_item() {
                if let Err(err) = library::set_keep(&item.file, !item.keep) {
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
                Dialog::plain(
                    tl!("replay-library-delete-title"),
                    tl!("replay-library-delete-confirm"),
                )
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
            if let Some(item) = self.selected_item() {
                self.start_export(item.file.clone());
            }
            return Ok(true);
        }

        let list_y = action_y + 0.10;
        let row_h = 0.09;
        for (index, button) in self.item_buttons.iter_mut().enumerate() {
            let y = list_y + index as f32 * row_h;
            if y + row_h > area.bottom() {
                break;
            }
            if button.touch(touch, t) {
                self.selected = Some(index);
                return Ok(true);
            }
        }
        let _ = button_w;
        let _ = action_w;
        Ok(false)
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

            let action_y = button_y + 0.09;
            let action_w = (area.w - 0.10) / 3.;
            let selected = self.selected_item();
            let keep_label = selected.map(|item| {
                if item.keep { tl!("replay-library-unkeep") } else { tl!("replay-library-keep") }
            }).unwrap_or_else(|| tl!("replay-library-select"));
            self.keep_btn.render_text(
                ui,
                Rect::new(area.x + 0.025, action_y, action_w, 0.07),
                t,
                keep_label,
                0.28,
                selected.is_some(),
            );
            self.delete_btn.render_text(
                ui,
                Rect::new(area.x + 0.05 + action_w, action_y, action_w, 0.07),
                t,
                tl!("replay-library-delete"),
                0.28,
                selected.is_some(),
            );
            self.export_btn.render_text(
                ui,
                Rect::new(area.x + 0.075 + action_w * 2., action_y, action_w, 0.07),
                t,
                tl!("replay-library-export"),
                0.28,
                selected.is_some(),
            );

            let mut y = action_y + 0.095;
            if let Some(error) = &self.error {
                ui.text(format!("{}: {error}", tl!("replay-library-error")))
                    .pos(area.x + 0.025, y)
                    .size(0.3)
                    .max_width(area.w - 0.05)
                    .draw();
                y += 0.08;
            }
            ui.text(format!("{} {}", self.items.len(), tl!("replay-library-count")))
                .pos(area.x + 0.025, y)
                .size(0.32)
                .draw();
            y += 0.055;

            if self.items.is_empty() {
                ui.text(tl!("replay-library-empty"))
                    .pos(area.x + 0.025, y)
                    .size(0.31)
                    .max_width(area.w - 0.05)
                    .draw();
            } else {
                let line_h = 0.09;
                let available = ((area.bottom() - y - 0.02) / line_h).floor().max(0.) as usize;
                for (index, item) in self.items.iter().take(available).enumerate() {
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
                            if item.keep { tl!("replay-library-kept") } else { tl!("replay-library-local") }
                        ),
                        0.26,
                        self.selected == Some(index),
                    );
                    y += line_h;
                }
                if self.items.len() > available {
                    ui.text(format!("{} {}", self.items.len() - available, tl!("replay-library-more")))
                        .pos(area.x + 0.025, area.bottom() - 0.04)
                        .size(0.25)
                        .draw();
                }
            }
        });
        Ok(())
    }
}
