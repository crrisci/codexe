# CodeXe modification notices

Copyright (c) 2026 Francesco Pio Crisci.
Native integration additions and modifications are licensed under Apache-2.0.
Upstream LICENSE, NOTICE and existing attributions are retained.

Compared with upstream rust-v0.162.1 (092d3acd6bec3e3a14bdc7e7a2810ab628ab759d),
the following files were added or modified for the native Xenolith integration.
This includes generated schema/SDK files and compressed schema exports, whose
formats or reproducible generators do not carry hand-written source comments.

- `codex-rs/Cargo.toml`
- `codex-rs/app-server-protocol/schema/json/ServerNotification.json`
- `codex-rs/app-server-protocol/schema/json/codex_app_server_protocol.schemas.json`
- `codex-rs/app-server-protocol/schema/json/codex_app_server_protocol.v2.schemas.json`
- `codex-rs/app-server-protocol/schema/json/v2/XenolithInferenceNotification.json`
- `codex-rs/app-server-protocol/schema/precomputed/app-server-exports-experimental.json.zst`
- `codex-rs/app-server-protocol/schema/precomputed/app-server-exports-stable.json.zst`
- `codex-rs/app-server-protocol/schema/typescript/ServerNotification.ts`
- `codex-rs/app-server-protocol/schema/typescript/ServerNotificationEnvelope.ts`
- `codex-rs/app-server-protocol/schema/typescript/v2/XenolithInferenceNotification.ts`
- `codex-rs/app-server-protocol/schema/typescript/v2/index.ts`
- `codex-rs/app-server-protocol/src/protocol/common.rs`
- `codex-rs/app-server-protocol/src/protocol/v2/model.rs`
- `codex-rs/app-server/src/bespoke_event_handling.rs`
- `codex-rs/app-server/src/notification_media.rs`
- `codex-rs/codex-api/src/common.rs`
- `codex-rs/config/src/thread_config/remote.rs`
- `codex-rs/core/Cargo.toml`
- `codex-rs/core/config.schema.json`
- `codex-rs/core/src/client.rs`
- `codex-rs/core/src/compact.rs`
- `codex-rs/core/src/lib.rs`
- `codex-rs/core/src/session/turn.rs`
- `codex-rs/core/src/turn_timing.rs`
- `codex-rs/core/src/xenolith.rs`
- `codex-rs/model-provider-info/src/lib.rs`
- `codex-rs/model-provider-info/src/model_provider_info_tests.rs`
- `codex-rs/otel/src/events/session_telemetry.rs`
- `codex-rs/protocol/src/protocol.rs`
- `codex-rs/rollout-trace/src/protocol_event.rs`
- `codex-rs/rollout/src/policy.rs`
- `codex-rs/tui/src/app/app_server_event_targets.rs`
- `codex-rs/tui/src/chatwidget/protocol.rs`
- `codex-rs/tui/src/chatwidget/tests.rs`
- `codex-rs/xenolith-wire/.gitignore`
- `codex-rs/xenolith-wire/Cargo.toml`
- `codex-rs/xenolith-wire/src/backend.rs`
- `codex-rs/xenolith-wire/src/bin/xenolith-session.rs`
- `codex-rs/xenolith-wire/src/lib.rs`
- `codex-rs/xenolith-wire/src/protocol.rs`
- `codex-rs/xenolith-wire/tests/native.rs`
- `sdk/python/src/openai_codex/generated/notification_registry.py`
- `sdk/python/src/openai_codex/generated/v2_all.py`

Additional fork-specific files: README.md, README-UPSTREAM.md, .gitignore, XENOLITH.md,
XENOLITH-MODIFICATIONS.md, xenolith/tests/, and
.github/workflows/xenolith-native.yml. Their additions are Apache-2.0.

README-UPSTREAM.md is an unchanged copy of the baseline upstream README, retained for reference.

CodeXe branding updates rename the fork and its documentation links to
`crrisci/codexe`. The executable, configuration keys and upstream package names
remain compatible with Codex. The name alludes to Intel Xe; the project is
independent of OpenAI and Intel.

Native reasoning and generation settings also add `codex-rs/core/src/xenolith/config.rs`.
These additions and their regression tests retain the Apache-2.0 license.
