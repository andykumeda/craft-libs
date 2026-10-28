# Craft native assistant

Portable desktop conversations backed by `craft-ai-core`. This crate owns no application
engine, GUI, document path, or MCP command policy. An app provides `ToolHost`; WordCraft is
its first integration. `craft-ui::assistant` provides optional egui widgets separately.

## Integration

1. Load the suite `StoreFile` through `storage::SharedStore`. Register the declarations from
   `AssistantRole::{Chat, Control}::declaration()` and save the user's bindings. Use
   `resolve_choices` for per-app overrides followed by suite defaults; broken overrides fail
   rather than routing to a different provider.
2. Build `HttpProvider` with an `Arc<dyn secrets::SecretStore>`, or a `codex::CodexProvider`.
   A small application/provider dispatcher chooses the adapter from each `Connection.provider`.
   Unknown adapters fail explicitly. Both implement the shared `ChatProvider` trait.
3. Start a worker with a `TurnRequest`, one shared `Cancellation`, and the app's `ToolHost`.
   `run_turn` emits `TurnEvent` values and returns the canonical conversation. A GUI host uses
   request/reply channels to handle `ToolHost` calls on its UI thread; it must validate the
   opaque `Target` and file grants before every operation. It must also check cancellation
   before dispatch, so closing a reply channel does not accidentally execute a late edit.
4. Persist each `TurnEvent::Checkpoint` synchronously in the app's `HistoryStore` before the
   callback returns; cancel the shared token immediately if saving fails. Checkpoints precede
   host dispatch and follow each result. Save an active conversation as interrupted. Poll other
   events into the UI. The runtime never restarts a persisted turn. Recovery snapshots pair all
   delegation calls with results, preserve completed/uncertain tool outcomes and can be reopened
   without executing a pending operation.

General chat receives only `control_app`, which delegates document-related requests to the
control binding. The control binding receives the host's tools. Tool-call IDs and their
results remain paired across continuation requests. Successful delegation results retain a
bounded command/result activity log. An oversized result records the completed command identity
without its body and stops the turn. The entire provider tool-call batch is validated before any
command is dispatched. Only bounded transcript text is shared
with the control model; model settings and document targets cannot be changed by a tool call.
Relevant document content may reach both configured providers and remain in local history.

A host's `Network` error means the result is uncertain and stops the turn; it is never retried.
Ordinary refused operations are returned to the model as errors. Previously completed actions
remain completed when a later call fails or the user presses Stop. The host owns Undo.

## Providers and capabilities

The HTTP adapter implements streaming OpenAI-compatible Chat Completions and `GET /models`.
Configure the API base including its version path, such as the provider's `/v1` base. Existing
AI-core presets cover OpenAI, OpenRouter, Ollama, LM Studio, and custom endpoints. Redirects and
HTTP retries are disabled. Known credentials are redacted from response text across streaming
fragment boundaries, returned tool arguments, model catalogs and errors. Remote HTTP and endpoint credentials/query strings are refused by
the shared connection validation. Secrets are resolved only at request construction.

A model catalog is not proof of tool support. Discovery marks tools supported only when the
provider advertises them. `verify_model` performs a synthetic `craft_connection_probe` call
without any document or host access. A manually entered model can be declared with
`craft_ai_core::assistant::tool_model` only after the user explicitly asserts tool support.
Unsupported requests and missing adapters fail without a provider or model fallback.

## Configuration

`RuntimeConfig` is the single source of defaults. The shared native configuration persists
validated overrides:

| Setting | Default | Accepted range |
| --- | --- | --- |
| Connect timeout | 15 seconds | 1–300 |
| Request timeout | 180 seconds | 1–3,600 |
| Model rounds per user turn, both roles combined | 12 | 1–100 |
| Provider response bytes | 8 MiB | 1 KiB–64 MiB |
| Serialized context bytes | 256 KiB | 1 KiB–4 MiB |
| Tools per response/catalog | 128 | 1–1,024 |
| Tool argument bytes | 64 KiB | 256 B–1 MiB |

Bindings start unset. Saved conversations cannot supply system instructions or unmatched/misplaced
tool results. Tool schemas count toward the configured context limit. Empty provider responses are explicit
errors. Long conversations fail with an actionable context-limit message rather
than silently dropping context. Start a new chat or explicitly increase the limit.

## Verification

`cargo test -p craft-ai-core -p craft-ai-native` covers contract compatibility, two independent
synthetic app catalogs, role routing, cancellation, duplicate-call prevention, fragmented
UTF-8/SSE/tool arguments, response limits, credential handling, and storage behavior. No
production credentials or private documents are needed. Live Codex containment, real provider
inference, and platform credential stores require separate acceptance evidence.

Storage locations, Codex setup, isolation behavior and live acceptance evidence are recorded in
[Assistant native services](../../docs/assistant-native-services.md).
