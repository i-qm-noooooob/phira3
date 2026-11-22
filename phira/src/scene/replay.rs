//! Local-only replay playback. It reuses the regular chart renderer and never
//! enters the ranked gameplay/update or score-submission path.

prpr_l10n::tl_file!("settings");

use crate::{
    replay::{
        library,
        storage::{ReplayChartRef, ReplayTape},
        transport,
    },
    scene::fs_from_path,
};
use anyhow::{Context, Result};
use macroquad::prelude::*;
use prpr::{
    config::Mods,
    core::NoteKind,
    ext::BLACK_TEXTURE,
    judge::{JudgeStatus, Judgement},
    scene::{GameMode, GameScene, LoadingScene, NextScene, Scene},
    time::TimeManager,
    ui::{DRectButton, Ui},
};
use std::collections::HashMap;

pub struct ReplayScene {
    game: GameScene,
    tape: ReplayTape,
    current: f64,
    last_real: f64,
    playing: bool,
    judge_index: usize,
    touch_index: usize,
    current_touches: HashMap<u64, (Vec2, usize)>,
    next_touch: Vec<Option<usize>>,
    play_button: DRectButton,
    back_button: DRectButton,
    backward_button: DRectButton,
    forward_button: DRectButton,
    mark_a_button: DRectButton,
    mark_b_button: DRectButton,
    loop_button: DRectButton,
    rate_button: DRectButton,
    playback_rate: f64,
    loop_a: Option<f64>,
    loop_b: Option<f64>,
    loop_enabled: bool,
    step_back_button: DRectButton,
    step_forward_button: DRectButton,
    prev_error_button: DRectButton,
    next_error_button: DRectButton,
    timeline_button: DRectButton,
    timeline_drag_id: Option<u64>,
    timeline_drag_target: Option<f64>,
    errors: Vec<f64>,
    selected_error: Option<usize>,
    exit: bool,
}

impl ReplayScene {
    pub async fn new(file: String) -> Result<Self> {
        let tape = library::load(&file).context("Cannot read replay")?;
        anyhow::ensure!(!tape.judges.is_empty(), "Replay has no per-note judgement events and cannot be played");

        let item = library::list()?
            .into_iter()
            .find(|item| item.file == file)
            .context("Replay is no longer in the local library")?;
        let candidates = crate::get_data().charts.iter().map(|chart| chart.local_path.clone()).collect::<Vec<_>>();
        let local_path = match library::match_chart(&tape, item.local_path.as_deref(), &candidates).await? {
            library::Match::Found(path, verified) => {
                if !verified {
                    prpr::scene::show_message("Legacy replay chart link is unverified").warn();
                }
                path
            }
            library::Match::Missing => anyhow::bail!("The chart required by this replay is not available locally"),
            library::Match::WrongVersion => anyhow::bail!("The local chart version does not match this replay"),
            library::Match::WrongAudio => anyhow::bail!("The chart matches, but its audio version does not"),
            library::Match::WrongServer => anyhow::bail!("This replay belongs to a different Phira server"),
        };

        let mut fs = fs_from_path(&local_path)?;
        let mut info = prpr::fs::load_info(fs.as_mut()).await?;
        if let Some(expected) = &tape.meta.fingerprint {
            anyhow::ensure!(library::fingerprint(fs.as_mut(), &info).await? == *expected, "Chart fingerprint mismatch");
        }
        if let Some(expected) = &tape.meta.audio_fingerprint {
            anyhow::ensure!(library::audio_fingerprint(fs.as_mut(), &info).await? == *expected, "Audio fingerprint mismatch");
        }
        if let Some(ReplayChartRef::Id(id)) = &tape.chart {
            info.id = Some(*id);
        }

        let mut config = crate::get_data().config.clone();
        config.speed = tape.speed.clamp(0.05, 16.);
        config.offset = tape.offset;
        config.mods = Mods::from_bits(tape.mods as i32).unwrap_or_default();
        let nightcore = config.mods.contains(Mods::NIGHTCORE);
        if tape.source_version < 5 && nightcore {
            config.speed *= 1.5;
        }
        config.mods.remove(Mods::NIGHTCORE);
        config.player_name = tape.meta.player.clone().unwrap_or_else(|| "REPLAY".to_owned());
        config.interactive = false;
        config.touch_debug = false;
        if let Some(aspect) = tape.aspect_ratio {
            config.aspect_ratio = Some(aspect);
            info.force_aspect_ratio = true;
        }

        let (background, illustration) = match LoadingScene::load(fs.as_mut(), &info.illustration, config.gameplay_background_blur).await {
            Ok((illustration, background, _)) => (background, illustration),
            Err(_) => (BLACK_TEXTURE.clone(), BLACK_TEXTURE.clone()),
        };
        let mut game = GameScene::new(GameMode::View, info, config, fs, None, background, illustration, None, None, None).await?;
        game.res.config.mods.set(Mods::NIGHTCORE, nightcore);

        anyhow::ensure!(
            tape.judges.iter().all(|event| {
                game.chart.lines.get(event.line as usize).is_some_and(|line| {
                    line.notes
                        .get(event.note as usize)
                        .is_some_and(|note| !matches!(event.kind, 5 | 6) || matches!(&note.kind, NoteKind::Hold { .. }))
                })
            }),
            "Replay judgement indices or hold events do not match the selected chart"
        );

        let mut next_touch = vec![None; tape.touches.len()];
        let mut next_by_id = HashMap::new();
        for (index, event) in tape.touches.iter().enumerate().rev() {
            next_touch[index] = next_by_id.insert(event.id, index);
        }

        let errors = tape
            .judges
            .iter()
            .filter(|event| matches!(event.kind, 2 | 3 | 8 | 9))
            .map(|event| event.t)
            .collect();

        Ok(Self {
            game,
            tape,
            current: 0.,
            last_real: 0.,
            playing: false,
            judge_index: 0,
            touch_index: 0,
            current_touches: HashMap::new(),
            next_touch,
            play_button: DRectButton::new(),
            back_button: DRectButton::new(),
            backward_button: DRectButton::new(),
            forward_button: DRectButton::new(),
            mark_a_button: DRectButton::new(),
            mark_b_button: DRectButton::new(),
            loop_button: DRectButton::new(),
            rate_button: DRectButton::new(),
            playback_rate: 1.0,
            loop_a: None,
            loop_b: None,
            loop_enabled: false,
            step_back_button: DRectButton::new(),
            step_forward_button: DRectButton::new(),
            prev_error_button: DRectButton::new(),
            next_error_button: DRectButton::new(),
            timeline_button: DRectButton::new(),
            timeline_drag_id: None,
            timeline_drag_target: None,
            errors,
            selected_error: None,
            exit: false,
        })
    }

    fn apply_events(&mut self) {
        while let Some(event) = self.tape.judges.get(self.judge_index).copied() {
            if event.t > self.current {
                break;
            }
            self.judge_index += 1;
            if matches!(event.kind, 5 | 6) {
                if let Some(note) = self
                    .game
                    .chart
                    .lines
                    .get_mut(event.line as usize)
                    .and_then(|line| line.notes.get_mut(event.note as usize))
                {
                    // Hold head events establish the in-progress hold state.
                    // The later tail event carries the one final score judgement.
                    note.judge = JudgeStatus::Hold(event.kind == 5, event.t, 0., false, f64::INFINITY);
                }
                continue;
            }
            let grade = match event.kind {
                0 | 4 => Judgement::Perfect,
                1 | 7 => Judgement::Good,
                2 | 8 | 9 => Judgement::Bad,
                3 => Judgement::Miss,
                _ => continue,
            };
            if let Some(note) = self
                .game
                .chart
                .lines
                .get_mut(event.line as usize)
                .and_then(|line| line.notes.get_mut(event.note as usize))
            {
                note.judge = JudgeStatus::Judged;
                self.game.judge.commit(event.t, grade, event.line, event.note, event.diff as f64);
            }
        }
    }

    fn apply_touches(&mut self) {
        while let Some(event) = self.tape.touches.get(self.touch_index).copied() {
            if event.t as f64 > self.current {
                break;
            }
            self.touch_index += 1;
            if event.phase >= 2 {
                self.current_touches.remove(&event.id);
            } else {
                self.current_touches.insert(event.id, (vec2(event.x, event.y), self.touch_index - 1));
            }
        }
    }

    fn touch_position(&self, mut pos: Vec2, index: usize) -> Vec2 {
        if let Some(next) = self.next_touch.get(index).copied().flatten() {
            let a = self.tape.touches[index];
            let b = self.tape.touches[next];
            if b.phase != 0 && b.t > a.t {
                let progress = ((self.current - a.t as f64) / (b.t - a.t) as f64).clamp(0., 1.) as f32;
                pos = pos.lerp(vec2(b.x, b.y), progress);
            }
        }
        pos
    }

    fn navigate_error(&mut self, next: bool) -> Result<()> {
        let Some(index) = transport::error_index(&self.errors, self.current, next, self.selected_error) else {
            return Ok(());
        };
        self.selected_error = Some(index);
        let target = (self.errors[index] - 1.0).max(0.);
        self.seek_by(target - self.current)
    }

    fn toggle_playback(&mut self) -> Result<()> {
        if self.playing {
            self.game.music.pause()?;
            self.playing = false;
        } else {
            if self.current >= self.game.res.track_length.max(0.) {
                let rewind = -self.current;
                self.seek_by(rewind)?;
            }
            let target = (self.current + self.game.offset() as f64).max(0.);
            self.game.music.seek_to(target)?;
            self.game.music.play()?;
            self.playing = true;
        }
        Ok(())
    }

    fn timeline_target(&self, touch: &Touch) -> Option<f64> {
        self.timeline_button
            .inner
            .normalized_x(touch.position)
            .and_then(|fraction| transport::timeline_target(fraction, 0., 1., self.game.res.track_length.max(0.)))
    }

    fn seek_by(&mut self, delta: f64) -> Result<()> {
        let target = (self.current + delta).clamp(0., self.game.res.track_length.max(0.));
        let was_playing = self.playing;
        self.game.music.pause()?;
        if target < self.current {
            self.game.reset_replay();
            self.judge_index = 0;
            self.touch_index = 0;
            self.current_touches.clear();
        }
        self.current = target;
        self.game.res.time = target;
        self.apply_events();
        self.apply_touches();
        self.game.tick_replay();
        let audio_time = (target + self.game.offset() as f64).max(0.);
        self.game.music.seek_to(audio_time)?;
        if was_playing {
            self.game.music.play()?;
        }
        Ok(())
    }
}

impl Scene for ReplayScene {
    fn enter(&mut self, tm: &mut TimeManager, target: Option<RenderTarget>) -> Result<()> {
        self.game.enter(tm, target)?;
        // Move the regular GameScene through its intro states once. Thereafter
        // playback owns its clock and never calls the live judgement/update loop.
        tm.seek_to(GameScene::BEFORE_TIME + 0.1);
        self.game.update(tm)?;
        tm.seek_to(0.1);
        self.game.update(tm)?;
        self.game.music.pause()?;
        let initial_audio_time = (self.game.offset() as f64).max(0.);
        self.game.music.seek_to(initial_audio_time)?;
        self.game.res.time = 0.;
        self.game.tick_replay();
        self.last_real = tm.real_time();
        Ok(())
    }

    fn touch(&mut self, tm: &mut TimeManager, touch: &Touch) -> Result<bool> {
        let t = tm.now() as f32;
        if self.back_button.touch(touch, t) {
            self.game.music.pause()?;
            self.playing = false;
            self.exit = true;
            return Ok(true);
        }
        match touch.phase {
            TouchPhase::Started if self.timeline_button.inner.contains(touch.position) => {
                self.timeline_drag_id = Some(touch.id);
                self.timeline_drag_target = self.timeline_target(touch);
                return Ok(true);
            }
            TouchPhase::Moved | TouchPhase::Stationary if self.timeline_drag_id == Some(touch.id) => {
                self.timeline_drag_target = self.timeline_target(touch);
                return Ok(true);
            }
            TouchPhase::Ended if self.timeline_drag_id == Some(touch.id) => {
                self.timeline_drag_id = None;
                self.timeline_drag_target = self.timeline_target(touch).or(self.timeline_drag_target);
                if let Some(target) = self.timeline_drag_target.take() {
                    self.seek_by(target - self.current)?;
                }
                return Ok(true);
            }
            TouchPhase::Cancelled if self.timeline_drag_id == Some(touch.id) => {
                self.timeline_drag_id = None;
                self.timeline_drag_target = None;
                return Ok(true);
            }
            _ => {}
        }
        if self.backward_button.touch(touch, t) {
            self.seek_by(-5.)?;
            return Ok(true);
        }
        if self.forward_button.touch(touch, t) {
            self.seek_by(5.)?;
            return Ok(true);
        }
        if self.step_back_button.touch(touch, t) {
            let (target, _) = transport::frame_step(&self.tape.frames, self.current, -1, self.game.res.track_length.max(0.));
            self.seek_by(target - self.current)?;
            return Ok(true);
        }
        if self.step_forward_button.touch(touch, t) {
            let (target, _) = transport::frame_step(&self.tape.frames, self.current, 1, self.game.res.track_length.max(0.));
            self.seek_by(target - self.current)?;
            return Ok(true);
        }
        if self.prev_error_button.touch(touch, t) {
            self.navigate_error(false)?;
            return Ok(true);
        }
        if self.next_error_button.touch(touch, t) {
            self.navigate_error(true)?;
            return Ok(true);
        }
        if self.mark_a_button.touch(touch, t) {
            self.loop_a = Some(self.current);
            self.loop_enabled = false;
            return Ok(true);
        }
        if self.mark_b_button.touch(touch, t) {
            self.loop_b = Some(self.current);
            self.loop_enabled = false;
            return Ok(true);
        }
        if self.rate_button.touch(touch, t) {
            self.playback_rate = transport::next_rate(self.playback_rate);
            self.game.set_replay_playback_rate(self.playback_rate, self.current, self.playing)?;
            return Ok(true);
        }
        if self.loop_button.touch(touch, t) {
            self.loop_enabled = !self.loop_enabled && self.loop_a.is_some_and(|a| self.loop_b.is_some_and(|b| b - a >= 0.05));
            if self.loop_enabled && self.current >= self.loop_b.unwrap_or(f64::INFINITY) {
                if let Some(target) = transport::loop_target(self.current, self.loop_a, self.loop_b, true) {
                    self.seek_by(target - self.current)?;
                }
            }
            return Ok(true);
        }
        if self.play_button.touch(touch, t) {
            self.toggle_playback()?;
            return Ok(true);
        }
        Ok(false)
    }

    fn update(&mut self, tm: &mut TimeManager) -> Result<()> {
        let real = tm.real_time();
        let delta = (real - self.last_real).clamp(0., 0.1);
        self.last_real = real;
        if self.playing {
            self.current += delta * self.game.res.config.speed as f64 * self.playback_rate;
            if let Some(target) = transport::loop_target(self.current, self.loop_a, self.loop_b, self.loop_enabled) {
                self.seek_by(target - self.current)?;
            }
            let end = self.game.res.track_length.max(0.);
            if self.current >= end {
                self.current = end;
                self.game.music.pause()?;
                self.playing = false;
            }
        }
        self.game.res.time = self.current;
        tm.seek_to(self.current + self.game.offset() as f64);
        self.apply_events();
        self.apply_touches();
        self.game.tick_replay();
        Ok(())
    }

    fn render(&mut self, tm: &mut TimeManager, ui: &mut Ui) -> Result<()> {
        self.game.render(tm, ui)?;
        if !self.current_touches.is_empty() {
            let fitted = self.game.res.camera.viewport.unwrap_or(ui.viewport);
            let mut touch_ui = Ui::new(ui.text_painter, Some(fitted));
            set_camera(&touch_ui.camera());
            let config = &self.game.res.config;
            for (pos, index) in self.current_touches.values() {
                let pos = self.touch_position(*pos, *index);
                touch_ui.fill_circle(
                    pos.x,
                    pos.y,
                    config.touch_point_size,
                    Color {
                        a: config.touch_point_alpha,
                        ..BLUE
                    },
                );
            }
            set_camera(&ui.camera());
        }
        let area = ui.content_rect();
        let y = area.y + area.h - 0.09;
        let marker_y = y - 0.075;
        let marker_w = (area.w - 0.12) / 4.;
        let tool_y = marker_y - 0.065;
        let tool_w = (area.w - 0.10) / 4.;
        self.step_back_button
            .render_text(ui, Rect::new(area.x + 0.025, tool_y, tool_w, 0.05), tm.now() as f32, "−1f", 0.22, true);
        self.step_forward_button
            .render_text(ui, Rect::new(area.x + 0.05 + tool_w, tool_y, tool_w, 0.05), tm.now() as f32, "+1f", 0.22, true);
        self.prev_error_button
            .render_text(ui, Rect::new(area.x + 0.075 + tool_w * 2., tool_y, tool_w, 0.05), tm.now() as f32, "‹ Error", 0.22, true);
        self.next_error_button
            .render_text(ui, Rect::new(area.x + 0.10 + tool_w * 3., tool_y, tool_w, 0.05), tm.now() as f32, "Error ›", 0.22, true);
        self.mark_a_button.render_text(
            ui,
            Rect::new(area.x + 0.025, marker_y, marker_w, 0.055),
            tm.now() as f32,
            format!("A {}", self.loop_a.map(|v| format!("{v:.1}s")).unwrap_or_else(|| "—".to_owned())),
            0.23,
            true,
        );
        self.mark_b_button.render_text(
            ui,
            Rect::new(area.x + 0.05 + marker_w, marker_y, marker_w, 0.055),
            tm.now() as f32,
            format!("B {}", self.loop_b.map(|v| format!("{v:.1}s")).unwrap_or_else(|| "—".to_owned())),
            0.23,
            true,
        );
        self.loop_button.render_text(
            ui,
            Rect::new(area.x + 0.075 + marker_w * 2., marker_y, marker_w, 0.055),
            tm.now() as f32,
            if self.loop_enabled { "A–B ON" } else { "A–B OFF" },
            0.23,
            self.loop_a.is_some_and(|a| self.loop_b.is_some_and(|b| b - a >= 0.05)),
        );
        self.rate_button.render_text(
            ui,
            Rect::new(area.x + 0.10 + marker_w * 3., marker_y, marker_w, 0.055),
            tm.now() as f32,
            format!("{:.2}x", self.playback_rate),
            0.23,
            true,
        );
        let w = (area.w - 0.14) / 4.;
        self.back_button
            .render_text(ui, Rect::new(area.x + 0.02, y, w, 0.07), tm.now() as f32, tl!("replay-back"), 0.28, true);
        self.backward_button
            .render_text(ui, Rect::new(area.x + 0.06 + w, y, w, 0.07), tm.now() as f32, "-5s", 0.28, true);
        self.play_button.render_text(
            ui,
            Rect::new(area.x + 0.10 + w * 2., y, w, 0.07),
            tm.now() as f32,
            if self.playing { tl!("replay-pause") } else { tl!("replay-play") },
            0.28,
            true,
        );
        self.forward_button
            .render_text(ui, Rect::new(area.x + 0.14 + w * 3., y, w, 0.07), tm.now() as f32, "+5s", 0.28, true);
        let total = self.game.res.track_length.max(0.001);
        let timeline_time = self.timeline_drag_target.unwrap_or(self.current);
        let timeline_progress = (timeline_time / total).clamp(0., 1.);
        self.timeline_button.render_text(
            ui,
            Rect::new(area.x + 0.025, y - 0.035, area.w - 0.05, 0.03),
            tm.now() as f32,
            format!("{:.1}s / {:.1}s  ·  {:.0}%", timeline_time, total, timeline_progress * 100.),
            0.22,
            false,
        );
        Ok(())
    }

    fn next_scene(&mut self, _tm: &mut TimeManager) -> NextScene {
        if self.exit {
            NextScene::Pop
        } else {
            NextScene::None
        }
    }
}
