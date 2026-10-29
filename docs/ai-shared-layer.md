# Suite-wide AI contract for app maintainers

**Status:** implementation in progress, 2026-10-28. `craft-ai-core` supplies the wasm-safe model;
`craft-ai-native` now supplies the desktop assistant runtime, connections store, credentials,
HTTP/Codex adapters and app-local conversations; `craft-ui::assistant` supplies optional egui
widgets. WordCraft integration is being verified separately. Runtime availability is not proof
that every provider or platform has passed live acceptance; see
[the implementation ledger](assistant-implementation.md).

## Ownership boundary

| Shared code | Each app or launcher |
| --- | --- |
| Connections, models, role declarations, suite defaults and app overrides | App identity and tool contracts |
| Streaming providers, bounded two-model routing, cancellation | Document targeting, authorization, mutation and Undo |
| Locked connection store, OS credential references, app-local history | Where the pane appears and which files the user grants |
| Transcript, composer, settings, model picker and activity widgets | Theme/translation inputs and handling emitted UI actions |

The assistant introduces no dependency on an app engine. A registered role is a declaration,
not installed code. A compatible model binding does not prove that a provider adapter exists
or that a model can call tools. Missing adapters and broken bindings fail explicitly; they do
not select another provider or model.

## Connections and roles

`craft_ai_core::StoreFile` remains the authoritative versioned connection format. An app uses
`craft_ai_native::storage::SharedStore::update` for locked, atomic changes and stores secrets
through `secrets::SecretStore`, never as fields in JSON or UI preference snapshots. A newer
store schema is refused without overwriting it. Old version-1 JSON without `custom_tools`
continues to load.

A feature declares a stable `RoleId`, capability and input set. Persist custom declarations in
`StoreFile.custom_tools`; validate before writing. `StoreFile::resolve` checks that an app has
not reinterpreted a shared role and applies per-app bindings before suite defaults. If an app
binds a role to an unavailable connection/model, that is an error, not a reason to use the
suite default instead.

The assistant's two declarations are `assistant.chat` and `assistant.control`. Use
`AssistantRole::declaration()` and native `resolve_choices`. Both need tool-capable text
models: general chat delegates through `control_app`; app control receives the host's MCP
catalog. A provider model listing does not establish tool support. `verify_model` performs a
synthetic, harmless tool-call probe; a manual model may be marked capable only through an
explicit user assertion. Settings identify the effective provider/model for each role.

## Porting an assistant to another Craft app

1. Adopt the same reviewed revisions of `craft-ai-core`, `craft-ai-native` and `craft-ui`.
   Keep native services target-gated; the core types and widget UI remain wasm-safe.
2. Supply an `AppId` and use `HistoryStore::new(shared_root, app_id)` to keep that app's chats
   separate. Reuse suite connections while permitting app-specific role overrides.
3. Implement `ToolHost::tools` and `ToolHost::call` against the app's existing MCP server.
   `Target` contains an opaque identity/revision; its meaning stays in the app. An egui app
   queues requests to its UI thread and returns the result to the worker. Preserve existing
   external MCP connections.
4. The host validates document identity, revision, selection and file grants before dispatch.
   It rejects changed/closed targets, checks cancellation again before edits, and reports
   uncertain outcomes as network errors so the runtime stops without replay. Normal refused
   operations are returned to the model. Completed edits remain in the app's normal Undo.
5. Start `run_turn` on a worker. Persist each `TurnEvent::Checkpoint` synchronously before
   returning from the callback, cancelling its shared token if saving fails. Checkpoints precede
   command dispatch and follow outcomes, preserve completed/uncertain calls and close unresolved
   delegations in recovery history. Feed other events to the pane and handle widgets' `Action`
   values. Stop shares one `Cancellation` with providers and host dispatch. Persist active turns
   as interrupted; never resume them on load.
6. Test with synthetic document data, including two host catalogs, wrong/closed targets,
   concurrent user edits, denied paths, cancellation before dispatch and provider failure
   after an earlier successful edit. Then run that app's ordinary build/UI/platform gates.

Provider interfaces, configuration defaults and examples are documented in
[`craft-ai-native`](../crates/ai-native/README.md). The executable portability regression is
`crates/ai-native/tests/runtime.rs::same_runtime_drives_two_independent_app_catalogs`.

## Deliberate limits

`AiClient` and `ToolDesk` APIs described in earlier proposals are not shipped APIs. This
implementation exposes the small contracts above; it does not add arbitrary executable adapter
plugins, model installation, cloud-synchronized chats, or sibling-app feature integrations.
Codex support uses isolated Craft-managed authentication and must pass runtime compatibility
checks. Live provider and platform acceptance remain separately recorded gates.

## Revision history

| Date | Change |
| --- | --- |
| 2026-10-28 | Replace proposed API references with implemented assistant contracts and porting guidance |
| 2026-10-13 | Initial shared-layer proposal |

## 2026-10-29 assistant settings and HTTP diagnostics

Settings now prioritize the two model routes, with full-width fields and explicit Apply scope.
Connection setup and custom IDs/scope, key headers, deletion and tool verification use expandable
sections. Existing action/persistence contracts and saved values are preserved; opening a section
never calls a provider. Hosts embedding the shared widget need no new API.

HTTP failures name the failed route and give owned status-specific guidance. For Ollama's known
context-overflow JSON, only validated token counts are displayed. Error-body reads are bounded by
`RuntimeConfig.max_response_bytes` and cancellable; other provider text is never echoed.
A synthetic sample continuation reproduced 4,432 tokens against a configured 4,096-token window.
No model selection, server configuration, provider fallback or retry behavior changed.

Verification: full UI library tests **464 passed / 2 ignored**, final focused assistant tests
**9 passed / 2 ignored**, manual synthetic screenshot export **1 passed** with visual inspection,
HTTP tests **6 passed** (eight status cases, redaction, safe context parsing, oversized bodies and
cancellation), UI/native all-target Clippy, fmt and core/UI WASM checks passed. These checks do not
claim full-workspace testing, installed-app acceptance or another platform's live inference.
This is a shared source change; each app must pin the published revision before it receives it.
