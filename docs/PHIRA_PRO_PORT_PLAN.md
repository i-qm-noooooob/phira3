# Phira Pro integration plan

## Base and working branch

- Upstream: `TeamFlos/phira`
- Working fork: `i-qm-noooooob/phira3`
- Integration branch: `integration/phira-pro-port`
- Pro reference: `i-qm-noooooob/phira-pro-build`

The integration branch is based on the official Phira tree. Keep `main` unchanged.

## Non-negotiable compatibility rules

1. Preserve the official Phira score calculation, judgement pipeline, score submission protocol, and ranking behavior.
2. Do not import the Pro score-service endpoint or Pro-specific upload/client implementation.
3. Practice features that alter speed, judgement windows, timing, or gameplay modifiers must run as local/unranked sessions and must not submit ranked scores.
4. Preserve official multiplayer and monitor components even where the Pro tree does not contain equivalent files.
5. Preserve RPEN license and notice files if RPEN components are ported.
6. Release order: complete and validate the ARM64 build first. Only after the ARM64 feature set is stable, port and validate ARMv7 (32-bit) across Rust target, NDK, JNI dependencies, FFmpeg and native libraries; changing Gradle ABI filters alone is insufficient.

## Port order

1. Establish and verify the official baseline build.
2. Port isolated UI/configuration features that do not affect score or judgement.
3. Port visual effects, skins and resource-pack support.
4. Port replay features, ensuring replay compatibility and no score submission side effects.
5. Port practice features behind an explicit unranked-session boundary.
6. Port plugins and chart-video export after dependency and licensing review.
7. Complete Android integration and produce a tested ARM64 APK first.
8. After ARM64 is stable, add ARMv7 (32-bit) support and validate all native dependencies as a separate phase.
9. Run tests/builds and document device-level limitations.

## Current status

- Repository access verified: admin/push access to the working fork.
- Integration branch created from the fork's official-base `main`.
- Initial source inventory completed: 1,228 Pro tracked blobs, 574 Pro-only files, 206 changed common files, and 448 identical common files. This is an inventory, not a claim that every file has already received a manual line-by-line review.
- Implemented isolated, visual/settings-only features on this branch: FPS overlay and six positions; custom combo label; Perfect Combo label; gameplay background blur and dim; touch-point color/opacity/size; localized gameplay settings; Low/Standard/High quality presets; and an anti-aliasing selector (Off/FXAA/MSAA 2x).
- These changes intentionally leave the official judgement and score pipeline untouched. Practice-affecting and score-affecting Pro features have not been ported.
- Latest visual/settings validation before replay work: CI run [45008490282](https://github.com/i-qm-noooooob/phira3/actions/runs/45008490282) passed Tests, Format, Clippy Linux, Clippy macOS, and Clippy iOS (5/5).
- User-approved delivery order: finish ARM64 first; only then complete ARMv7 (32-bit). Neither APK/device validation phase has been completed yet.
- Performance-settings review found Pro-only shader and font-atlas optimizations require their full rendering/platform support; these are not being exposed as settings until that support is ported and validated.


## Replay feature audit (next major port)

The Pro replay implementation is a subsystem, not a standalone screen. The source inventory includes `phira/src/replay.rs`, `replay/capture.rs`, `replay/library.rs`, `replay/transport.rs`, `replay/analysis/*`, `page/replays.rs`, `page/replay_analysis.rs`, and replay scenes/locales. The regular branch currently has only the server record model at `phira/src/client/model/record.rs`; it does not contain the Pro replay subsystem.

Important compatibility constraints from the Pro replay design:
- Replay import must remain local-only. Imported records must not change PB, RKS, play count, official score history, or any upload queue.
- Playback must consume recorded judgement events; it must not rerun judgement using the viewer's current settings or write scores to the official service.
- Keep bounded parsing and validation. The documented format uses a 128 MiB input cap and supports multiple legacy versions; do not implement a permissive unbounded importer.
- Preserve old formats and distinguish missing event data from a valid zero-score replay. Never fabricate judgement or touch events from a final score.
- Replay capture/playback must be checked against the official game clock, pause/retry/end states, touch lifecycle, and chart/audio identity before release.

The first isolated storage slice is implemented in `phira/src/replay/storage.rs`: bounded decoding for JSON/Zstandard v5/v6, binary Pro v3/v4, and JPhiraRec v0/v1 (uncompressed, Zstandard, and DEFLATE). The parser enforces a 128 MiB input/decompression cap, 2,000,000-event caps, bounded ULEB128/string reads, and rejects malformed/truncated payloads. Legacy parsing normalizes data into JSON for inspection only; it is not yet wired to the complete Replay model or playback scene.

A local Replay Library slice is connected from Chart settings. It can import a single `.phirarec`, `.phirar`, or `.ppr` file, or a ZIP package; list valid local replay metadata; refresh; mark a file to keep; delete after confirmation; and export through the platform save picker. Imports are SHA-256 deduplicated and remain outside score history/upload paths. The index and archive readers are size/entry bounded. The library UI now has page navigation, a search field matching chart/player/filename, and sort modes for recent, chart name, and file size. A chart-match action compares chart/audio fingerprints where available and labels legacy path/ID matches as unverified. A first local playback scene is now wired to the library: it resolves and rechecks the chart/audio fingerprints, loads the chart in `GameMode::View`, plays the recorded audio, applies recorded note events through the regular Phira `Judge::commit`, and provides pause/play plus 5-second backward/forward seeking. Hold-head events restore hold state separately so the tail event does not double-count a note. The scene has no player/upload/save callbacks and does not enter the ranked play flow. Because regular Phira has four judgement grades, Pro-only Great/Ok/Meh grades are mapped to the nearest regular grade for the replay HUD; this is not an exact Pro grade renderer. Touch/finger visualization, A-B looping, frame-step, previous/next error navigation, and a separate replay playback-rate selector are now wired. Playback-rate changes recreate only the local replay audio renderer and keep the recorded chart speed separate; they do not touch the ranked score path. Replay Library has page navigation, metadata search, and sorting. The replay scene also has a draggable timeline seek control. The search/sort and timeline changes are awaiting the latest CI run. Still outstanding: Pro capture/settings/gameplay restoration, checkpoints for long backward seeks, missing-chart download, and exact Pro judgement rendering. A chart-match action has since been added; it compares chart/audio fingerprints where available and labels legacy path/ID matches as unverified.

The storage layer now also deserializes a typed replay preview model before library listing/import. It validates speed, offset, aspect ratio, frame timestamps, touch phases/coordinates, judgement kinds, head grades, and event limits; it records whether legacy event ordering needed repair and stably sorts frames/touches/judgements for later playback. Pro-only capture/settings/gameplay extension objects remain opaque. This is not yet the full Pro Replay model and does not instantiate a playback scene. The Pro reference playback uses `GameMode::View`, applies recorded judgement events to the existing chart/judge display state, and advances visual/touch state separately; the port must first add only the missing replay-view APIs, without copying Pro score-service or judgement-pipeline changes.

Replay playback baseline (commit `bf1c829f1ff549406f3d535cf83f3810239244d1`) passed all five CI jobs in run [45025446381](https://github.com/i-qm-noooooob/phira3/actions/runs/45025446381). Follow-up seek/reset work passed all five CI jobs in run [45025751880](https://github.com/i-qm-noooooob/phira3/actions/runs/45025751880). An additional validation now rejects hold-head events that point to non-hold chart notes; CI run [45026064452](https://github.com/i-qm-noooooob/phira3/actions/runs/45026064452) passed all five jobs. No Android APK or device test has been performed. This is an initial local replay player, not end-to-end Pro replay parity.

## Android APK pipeline (ARM64 first)

An initial APK packaging workflow now exists at `.github/workflows/build-android-arm64.yml`. It builds the current Rust `phira` crate for `aarch64-linux-android`, verifies the ELF architecture, replaces the native library inside the matching official v0.8.3 ARM64 APK shell, aligns and re-signs the APK, verifies the signature, and uploads the APK plus the native library and checksums as a GitHub Actions artifact.

The official Android shell expects the JNI `QuadNative.preprocessInput` hook. The branch now exports a no-op compatibility symbol for that hook; touch events continue through the existing miniquad `surfaceOnTouch` path. This matches the documented Phira Android build integration, but it still needs confirmation from the first Android CI run and an actual device launch.

The workflow uses a CI-generated signing key. Because the APK keeps the official application ID but is signed with a different certificate, it cannot update an already-installed official Phira build; uninstall the official build before installing this test APK. This key is ephemeral per workflow run, so these artifacts are test builds, not stable upgradeable releases. A persistent release keystore and a dedicated application ID remain release-engineering work.

The workflow currently targets ARM64 only and packages the regular official Android Java shell. Pro-only Android Java/RPEN/editor integrations are not yet ported into this shell. Do not call the Android deliverable complete until the workflow passes, the APK installs and launches, chart/audio/gameplay smoke tests pass, and the score/ranking behavior is confirmed unchanged. ARMv7 remains a separate phase after ARM64 stability.

FPS overlay follow-up: the overlay now samples Macroquad's frame delta directly and flushes pending gameplay rendering before drawing the text in the UI camera, then restores the prior camera state. New configurations default `show_fps` to true; existing users can still toggle it in Chart Settings. ARM64 build run [45075165607](https://github.com/i-qm-noooooob/phira3/actions/runs/45075165607) succeeded, and the five standard source checks in run [45075165541](https://github.com/i-qm-noooooob/phira3/actions/runs/45075165541) passed. The resulting test APK SHA-256 is `a9a83704ba900d1d38d469e2eabf9c73003e02cc951ef871e278a6d4412545dc`. Device-level FPS visibility remains unverified. The background audit workflow now installs ALSA and Wayland development packages so native Linux test dependencies are available.

