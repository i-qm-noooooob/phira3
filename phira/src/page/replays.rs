prpr_l10n::tl_file!("replays");

use super::{Page, SharedState};
use crate::replay::library::{self, ReplayItem};
use anyhow::Result;
use macroquad::prelude::*;
use prpr::{
    ext::{semi_black, RectExt},
    scene::request_file,
    ui::{DRectButton, Ui},
};
use std::{borrow::Cow, sync::atomic::{AtomicBool, Ordering}};

static DIRTY: AtomicBool = AtomicBool::new(false);

pub fn mark_dirty() {
    DIRTY.store(true, Ordering::Release);
}

pub struct ReplayLibraryPage {
    items: Vec<ReplayItem>,
    import_btn: DRectButton,
    refresh_btn: DRectButton,
    error: Option<String>,
}

impl ReplayLibraryPage {
    pub fn new() -> Self {
        let mut page = Self {
            items: Vec::new(),
            import_btn: DRectButton::new(),
            refresh_btn: DRectButton::new(),
            error: None,
        };
        page.reload();
        page
    }

    fn reload(&mut self) {
        match library::list() {
            Ok(items) => {
                self.items = items;
                self.error = None;
            }
            Err(err) => self.error = Some(err.to_string()),
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
        Ok(())
    }

    fn touch(&mut self, touch: &Touch, s: &mut SharedState) -> Result<bool> {
        if self.import_btn.touch(touch, s.t) {
            request_file("_import_replay");
            return Ok(true);
        }
        if self.refresh_btn.touch(touch, s.t) {
            self.reload();
            return Ok(true);
        }
        Ok(false)
    }

    fn render(&mut self, ui: &mut Ui, s: &mut SharedState) -> Result<()> {
        s.render_fader(ui, |ui| {
            let area = ui.content_rect().feather(-0.025);
            ui.fill_path(&area.rounded(0.02), semi_black(0.48));
            ui.text(tl!("replay-library-title"))
                .pos(area.x + 0.025, area.y + 0.025)
                .size(0.55)
                .draw();

            let button_y = area.y + 0.105;
            let button_w = (area.w - 0.08) / 2.;
            self.import_btn.render_text(
                ui,
                Rect::new(area.x + 0.025, button_y, button_w, 0.075),
                s.t,
                tl!("replay-library-import"),
                0.34,
                false,
            );
            self.refresh_btn.render_text(
                ui,
                Rect::new(area.x + 0.055 + button_w, button_y, button_w, 0.075),
                s.t,
                tl!("replay-library-refresh"),
                0.34,
                false,
            );

            let mut y = button_y + 0.105;
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
            y += 0.06;

            if self.items.is_empty() {
                ui.text(tl!("replay-library-empty"))
                    .pos(area.x + 0.025, y)
                    .size(0.31)
                    .max_width(area.w - 0.05)
                    .draw();
            } else {
                let line_h = 0.075;
                let available = ((area.bottom() - y - 0.02) / line_h).floor().max(0.) as usize;
                for item in self.items.iter().take(available) {
                    let title = item.chart_name.clone().unwrap_or_else(|| tl!("replay-library-untitled").into_owned());
                    let player = item.player.clone().unwrap_or_else(|| tl!("replay-library-unknown-player").into_owned());
                    ui.text(format!("{title} — {player}"))
                        .pos(area.x + 0.025, y)
                        .size(0.31)
                        .max_width(area.w - 0.05)
                        .draw();
                    ui.text(format!("v{} · {} bytes · {}", item.version, item.size, if item.keep { tl!("replay-library-kept") } else { tl!("replay-library-local") }))
                        .pos(area.x + 0.025, y + 0.033)
                        .size(0.24)
                        .color(Color::new(1., 1., 1., 0.65))
                        .draw();
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
