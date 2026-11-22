//! Local-only replay playback. It reuses the regular chart renderer and never
//! enters the ranked gameplay/update or score-submission path.

use crate::{
    replay::{
        library,
        storage::{ReplayChartRef, ReplayTape},
    },
    scene::fs_from_path,
};
use anyhow::{Context, Result};
use macroquad::prelude::*;
use prpr::{
    config::{Config, Mods},
    ext::BLACK_TEXTURE,
    judge::{JudgeStatus, Judgement},
    scene::{GameMode, GameScene, LoadingScene, NextScene, Scene},
    time::TimeManager,
    ui::{DRectButton, Ui},
};

pub struct ReplayScene {
    game: GameScene,
    tape: ReplayTape,
    current: f64,
    last_real: f64,
    playing: bool,
    judge_index: usize,
    play_button: DRectButton,
    back_button: DRectButton,
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

        let (background, illustration) = match LoadingScene::load(fs.as_mut(), &info.illustration).await {
            Ok((illustration, background, _)) => (background, illustration),
            Err(_) => (BLACK_TEXTURE.clone(), BLACK_TEXTURE.clone()),
        };
        let mut game = GameScene::new(GameMode::View, info, config, fs, None, background, illustration, None, None, None).await?;
        game.res.config.mods.set(Mods::NIGHTCORE, nightcore);

        anyhow::ensure!(
            tape.judges.iter().all(|event| game
                .chart
                .lines
                .get(event.line as usize)
                .is_some_and(|line| (event.note as usize) < line.notes.len())),
            "Replay judgement indices do not match the selected chart"
        );

        Ok(Self {
            game,
            tape,
            current: 0.,
            last_real: 0.,
            playing: false,
            judge_index: 0,
            play_button: DRectButton::new(),
            back_button: DRectButton::new(),
            exit: false,
        })
    }

    fn apply_events(&mut self) {
        while let Some(event) = self.tape.judges.get(self.judge_index).copied() {
            if event.t > self.current {
                break;
            }
            self.judge_index += 1;
            let grade = match event.kind {
                0 | 4 => Judgement::Perfect,
                1 | 7 => Judgement::Good,
                2 | 8 | 9 => Judgement::Bad,
                3 => Judgement::Miss,
                5 | 6 => match event.head_grade.unwrap_or(if event.kind == 5 { 0 } else { 1 }) {
                    0 | 4 => Judgement::Perfect,
                    1 | 7 => Judgement::Good,
                    2 | 8 | 9 => Judgement::Bad,
                    _ => Judgement::Miss,
                },
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

    fn toggle_playback(&mut self) -> Result<()> {
        if self.playing {
            self.game.music.pause()?;
            self.playing = false;
        } else {
            self.game.music.seek_to((self.current + self.game.offset() as f64).max(0.))?;
            self.game.music.play()?;
            self.playing = true;
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
        self.game.music.seek_to((self.game.offset() as f64).max(0.))?;
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
            self.current += delta * self.game.res.config.speed as f64;
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
        self.game.tick_replay();
        Ok(())
    }

    fn render(&mut self, tm: &mut TimeManager, ui: &mut Ui) -> Result<()> {
        self.game.render(tm, ui)?;
        let area = ui.content_rect();
        let y = area.y + area.h - 0.09;
        let w = (area.w - 0.08) / 2.;
        self.back_button
            .render_text(ui, Rect::new(area.x + 0.02, y, w, 0.07), tm.now() as f32, "Back", 0.3, true);
        self.play_button.render_text(
            ui,
            Rect::new(area.x + 0.06 + w, y, w, 0.07),
            tm.now() as f32,
            if self.playing { "Pause" } else { "Play" },
            0.3,
            true,
        );
        let total = self.game.res.track_length.max(0.001);
        let progress = (self.current / total).clamp(0., 1.);
        ui.text(format!("{:.1}s / {:.1}s  ·  {:.0}%", self.current, total, progress * 100.))
            .pos(area.x + 0.025, y - 0.035)
            .size(0.25)
            .draw();
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
