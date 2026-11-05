# Craft assistant

One independently built Rust helper supplies the assistant interface and providers to PDFCraft, WordCraft and GridCraft. Each desktop app launches its own helper session using private pipes. Nothing starts a server, registers MCP, or listens on a port.

The app retains document tools, target/selection guards, exact file grants and Undo. The helper reuses craft-ui assistant widgets and craft-ai-native providers, credential storage, cancellation and checkpointed recovery. Conversations stay app-separated; connections, authentication and the default model stay suite-wide. Optional app overrides remain explicit. Every turn reads the shared configuration; idle panels refresh saved choices without overwriting unsaved settings edits.

## Build and configure

```sh
cargo build -p craft-assistant --features runtime
cargo test -p craft-assistant --features runtime -j 2
```

Release builds must provide the pinned optional craft-fonts build input for supported interface scripts (`CRAFT_FONTS_DIR`, with `CRAFT_FONTS_REQUIRED=1`); fonts never enter this repository. Default dependency fonts and their licenses are recorded in the root ATTRIBUTION.md and NOTICE. This crate does not embed proprietary system fonts. Without the optional input, unsupported scripts may show missing glyphs. The helper embeds licensed faces from the pinned main manifest and, on native builds, `fonts/simplified-chinese-manifest.txt`. It excludes Source Sans 3, Source Serif 4, Source Han Sans/Serif and every Noto Sans/Serif/Mono CJK family to satisfy the strictest host policy. Permitted BIZ Japanese faces remain, and LXGW WenKai Lite supplies Simplified Chinese without depending on host fonts. Required native builds fail if the reviewed Simplified Chinese manifest is absent. Optional builds with older main-only checkouts warn, retain permitted main faces, and do not substitute a prohibited Chinese face.

See [installation and release configuration](../../docs/assistant-install.md) for shared `component.json`, publisher identity, exact platform asset selection, limits, signed `.app` packaging and rollback. `CRAFT_AI_CONFIG_DIR` overrides the existing suite settings root. Missing, newer or invalid configuration fails explicitly without choosing another source/provider.

Debug app builds alone allow the explicit absolute `CRAFT_ASSISTANT_EXECUTABLE` path for isolated development. This bypass is absent from shipping release builds. Set `CRAFT_AI_CONFIG_DIR` and `CODEX_HOME` to isolated temporary directories when testing; no automated test should use personal chats or credentials. The helper accepts only `--stdio`; its stdout is exclusively framed protocol data.

## Lifecycle and updates

The gear button opens assistant settings; shared version, update checks and explicit installation controls appear only while settings is open. If the helper is missing, recovery controls remain available without it. The Controller requires an explicit install action for the selected candidate. Download/staging/signature verification runs off the app UI thread. Installation never replaces app bundles or the executable of an existing session. Versions remain immutable; running turns finish on their original version.

When an idle panel next opens, the controller resolves the shared active version. It freezes new input, drains outstanding frames/input, then asks the existing helper for a fresh restart barrier. Only a safe reply permits replacement, with the active chat restored. Drafts, unsent file paths, unsaved settings, pending Send handoffs and active turns defer replacement. No interrupted tool operation is replayed.

A small versioned wire protocol carries explicit input/mesh/texture/platform types, service commands and app-authorized tools. It does not serialize egui internal layouts. The helper declares missing localization keys; each app supplies its actual translations, including when the selected language changes.

Existing automation commands remain available. Shared-mode adapters additionally expose `ui.assistant.updateCheck`, `ui.assistant.updateInstall`, `ui.assistant.updateStatus` and `ui.assistant.reloadComponent`; install is the explicit approval of the candidate from the preceding check. `Controller` also exposes those actions directly. App startup must explicitly call `enable_shared_assistant()`; that selects compatible customized app update sources. Library consumers retain their existing assistant behavior unless they opt in; shared mode never silently falls back to compiled-in providers.

## Acceptance

See the [implementation ledger](../../docs/independent-assistant.md). Synthetic private-pipe, model-sharing, input, accessibility-tree, cancellation, installer and update-policy tests do not establish native VoiceOver behavior, live provider authentication, or installed signed-update acceptance. No public release or production installation is performed by building this crate.

## Chat layout

The header has an explicit New chat action, a Chat history dropdown for saved conversations and a settings gear. Delete remains inside history.
The composer has a `+` menu for file selection or an explicit output path, a compact model
picker and Send/Stop. Attached paths remain visible and removable. Model choices use existing
streaming/tool capability validation and persist only after the host acknowledges the save;
quick choices apply to the current app, while settings retains suite defaults and custom IDs.

The assistant executable owns this UI and native provider services. Compatible helper releases
can update all host panels without rebuilding the apps. Host document engines, tool changes and
incompatible bridge changes still require app builds. This layout adds an optional
`settings_open` status field: existing hosts ignore it, but need a one-time rebuild to hide their
previous always-visible update toolbar. No protocol version or settings schema changed.

Older helpers that omit the optional settings-view field retain their existing update toolbar, so a new host can still update an old helper.

The compact-layout local rollout uses helper 0.1.1, retaining protocol/settings/history format1 and the immutable installed 0.1.0 for rollback.

The navigation refinement ships as helper0.1.2 without another host rebuild (protocol/settings/history remain1). Version, update button, result/status copy and install action are one settings-only group in compatible hosts; regression coverage checks all three labels together.

Idle panels check the shared connection/runtime files and app-specific history directory for changes before reloading them.
Unchanged settings keep the composer model picker enabled; explicit Reload settings and
actual shared-file changes retain the existing validated reload path.

Tool calls, results and activity details remain in assistant state but are hidden from the conversation display.


## Normal Codex environment and approvals

Helper0.1.5 source uses the current Codex profile/account and its enabled skills, plugins,
MCP servers and configured permissions. `CODEX_HOME` selects the profile, otherwise Codex's
normal `~/.codex` applies. HOME-based skills and the inherited launch working directory remain
available. Existing Codex configuration is not copied or overwritten. Settings identifies this
shared profile behavior; an existing ChatGPT sign-in is reused.

The pane shows native command/file/permission requests and input/MCP forms with explicit
responses and Stop. There are no automatic grants. Compatible hosts keep bridge1; helper-owned
rendering handles the new UI. Private-pipe automation can query `interaction_pending` and send
`respond` with the exact pending id and response. Never log request parameters or user answers.
App-engine document calls preserve Undo; external native shell/plugin changes follow Codex
policy and are not part of app Undo. See the current [native contract](../../docs/assistant-native-services.md#codex-adapter-helper-015-source).
