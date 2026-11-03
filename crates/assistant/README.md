# Craft assistant

One independently built Rust helper supplies the assistant interface and providers to PDFCraft, WordCraft and GridCraft. Each desktop app launches its own helper session using private pipes. Nothing starts a server, registers MCP, or listens on a port.

The app retains document tools, target/selection guards, exact file grants and Undo. The helper reuses craft-ui assistant widgets and craft-ai-native providers, credential storage, cancellation and checkpointed recovery. Conversations stay app-separated; connections, authentication and the default model stay suite-wide. Optional app overrides remain explicit. Every turn reads the shared configuration; idle panels refresh saved choices without overwriting unsaved settings edits.

## Build and configure

```sh
cargo build -p craft-assistant --features runtime
cargo test -p craft-assistant --features runtime -j 2
```

Release builds must provide the pinned optional craft-fonts build input for supported interface scripts (`CRAFT_FONTS_DIR`, with `CRAFT_FONTS_REQUIRED=1`); fonts never enter this repository. Default dependency fonts and their licenses are recorded in the root ATTRIBUTION.md and NOTICE. This crate does not embed proprietary system fonts. Without the optional input, unsupported scripts may show missing glyphs.

See [installation and release configuration](../../docs/assistant-install.md) for shared `component.json`, publisher identity, exact platform asset selection, limits, signed `.app` packaging and rollback. `CRAFT_AI_CONFIG_DIR` overrides the existing suite settings root. Missing, newer or invalid configuration fails explicitly without choosing another source/provider.

Debug app builds alone allow the explicit absolute `CRAFT_ASSISTANT_EXECUTABLE` path for isolated development. This bypass is absent from shipping release builds. Set `CRAFT_AI_CONFIG_DIR` to an isolated temporary directory when testing; no test should use personal chats or credentials. The helper accepts only `--stdio`; its stdout is exclusively framed protocol data.

## Lifecycle and updates

The Controller offers a shared update check and requires an explicit install action for the selected candidate. Download/staging/signature verification runs off the app UI thread. Installation never replaces app bundles or the executable of an existing session. Versions remain immutable; running turns finish on their original version.

When an idle panel next opens, the controller resolves the shared active version. It freezes new input, drains outstanding frames/input, then asks the existing helper for a fresh restart barrier. Only a safe reply permits replacement, with the active chat restored. Drafts, unsent file paths, unsaved settings, pending Send handoffs and active turns defer replacement. No interrupted tool operation is replayed.

A small versioned wire protocol carries explicit input/mesh/texture/platform types, service commands and app-authorized tools. It does not serialize egui internal layouts. The helper declares missing localization keys; each app supplies its actual translations, including when the selected language changes.

Existing automation commands remain available. Shared-mode adapters additionally expose `ui.assistant.updateCheck`, `ui.assistant.updateInstall`, `ui.assistant.updateStatus` and `ui.assistant.reloadComponent`; install is the explicit approval of the candidate from the preceding check. `Controller` also exposes those actions directly. App startup must explicitly call `enable_shared_assistant()`; that selects compatible customized app update sources. Library consumers retain their existing assistant behavior unless they opt in; shared mode never silently falls back to compiled-in providers.

## Acceptance

See the [implementation ledger](../../docs/independent-assistant.md). Synthetic private-pipe, model-sharing, input, accessibility-tree, cancellation, installer and update-policy tests do not establish native VoiceOver behavior, live provider authentication, or installed signed-update acceptance. No public release or production installation is performed by building this crate.
