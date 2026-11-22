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
6. Android ARMv7 support requires validating Rust target, NDK, JNI dependencies, FFmpeg and native libraries together; changing Gradle ABI filters alone is insufficient.

## Port order

1. Establish and verify the official baseline build.
2. Port isolated UI/configuration features that do not affect score or judgement.
3. Port visual effects, skins and resource-pack support.
4. Port replay features, ensuring replay compatibility and no score submission side effects.
5. Port practice features behind an explicit unranked-session boundary.
6. Port plugins and chart-video export after dependency and licensing review.
7. Port Android-specific features and validate both ARM64 and ARMv7.
8. Run tests/builds and document device-level limitations.

## Current status

- Repository access verified: admin/push access to the working fork.
- Integration branch created from the fork's official-base `main`.
- Initial source inventory completed: 1,228 Pro tracked blobs, 574 Pro-only files, 206 changed common files, and 448 identical common files. This is an inventory, not a claim that every file has already received a manual line-by-line review.
- Implemented isolated, visual/settings-only features on this branch: FPS overlay and six positions; custom combo label; Perfect Combo label; gameplay background blur and dim; touch-point color/opacity/size; localized gameplay settings; Low/Standard/High quality presets; and an anti-aliasing selector (Off/FXAA/MSAA 2x).
- These changes intentionally leave the official judgement and score pipeline untouched. Practice-affecting and score-affecting Pro features have not been ported.
- Latest visual/settings validation before replay work: CI run [45008490282](https://github.com/i-qm-noooooob/phira3/actions/runs/45008490282) passed Tests, Format, Clippy Linux, Clippy macOS, and Clippy iOS (5/5).
- Android APK build, ARMv7 native dependency validation, and physical-device testing have not yet been completed.
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

A local Replay Library slice is now connected from Chart settings. It can import a single `.phirarec`, `.phirar`, or `.ppr` file, or a ZIP package; list valid local replay metadata; refresh; mark a file to keep; delete after confirmation; and export through the platform save picker. Imports are SHA-256 deduplicated and remain outside score history/upload paths. The index and archive readers are size/entry bounded. The library UI currently shows only the first visible page of items and has no search/filter or playback action. It does not yet provide replay playback, chart matching, analysis, or replay capture.

Validation for the Replay Library slice: CI run [45021899921](https://github.com/i-qm-noooooob/phira3/actions/runs/45021899921) passed Format, Tests, Clippy Linux, Clippy macOS, and Clippy iOS (5/5). The run covered commit `ce7062dc6b178c6c356d3fc2ba2a92c93e3491cf`; later commits on the branch still require their own CI validation. No Android APK or ARMv7 device test has been performed. Do not treat this as end-to-end replay support.
