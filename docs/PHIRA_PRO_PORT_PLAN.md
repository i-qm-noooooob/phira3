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
- Initial source inventory completed; detailed file-by-file review remains part of each port stage.
- No application source changes or APK builds have been made yet.
