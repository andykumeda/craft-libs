# Assistant native services

Status: normal Codex environment and interactive approvals passed source verification for helper0.1.5; native acceptance remains in progress. Last checked: 2026-11-04. Installed delivery is recorded separately.

`craft-ai-native` keeps shared connections, local chat histories, credentials, and Codex process
management outside application engines. It imports the portable `craft-ai-core::assistant`
conversation contract. Apps remain responsible for document/file authorization and MCP dispatch.

## Storage and secrets

`SharedStore::default_root()` uses `CRAFT_AI_CONFIG_DIR` when explicitly set, otherwise the
platform configuration directory's `Craft/ai` folder. An empty override is an error. The store
keeps `connections.json` in the existing `StoreFile` format and `runtime.json` in the shared
`RuntimeConfig` format; defaults live in `craft-ai-core::assistant::RuntimeConfig` only. `load`,
`update`, `config`, and `update_config` use an OS file lock. Updates read the latest file under
the lock before applying the mutation, preserving another app's bindings. A newer unsupported
store version is left untouched. Writes use `craft-atomic` with private creation permissions.

`HistoryStore::new(root, app)` keeps conversations in `chats/<app>/`, separate from documents
and from other apps. `create`, `list`, `load`, `save`, and `delete` accept path-safe identifiers.
The caller saves `interrupted: true` before starting inference and clears it only after completion;
loading a conversation preserves that flag and never starts or replays a turn. Conversation
files are limited to 8 MiB, and listing refuses more than 10,000 conversations. These are file
format/resource ceilings; model request/context limits are separately configurable.

`SecretStore` provides `get`, `set`, and `delete` through an opaque `SecretRef`. `KeyringStore`
uses the OS store under `org.craft-apps.assistant`; unavailable or locked stores fail explicitly.
There is no plaintext fallback. Settings contain references only. Native code never logs an API
key. The storage API is blocking and must run on an application's worker, not its UI thread.

## Codex adapter (helper 0.1.5 source)

Route a `codex` connection to `CodexProvider::new(CodexOptions::from_environment())`.
The earlier `in_shared_root(root)` constructor remains compatible with existing hosts and
uses the same discovery; it now selects the normal
`CODEX_HOME`, or `~/.codex` when unset. `HOME` and the normal process environment remain intact
for Codex's global skills, installed plugins, MCP configuration, credentials and policies.
The workspace is independent of the helper's launch directory (including Finder's `/`).
`CRAFT_CODEX_WORKSPACE` selects an absolute project/output directory. Without it the helper uses
`<Craft AI config root>/workspaces/<app-id>`; standalone provider consumers use `workspaces/codex`
unless they supply an app default with `with_workspace`. Outputs persist across turns and restarts.
Root, relative, parent-traversal and application-bundle paths are refused, including existing
symlink targets; invalid overrides fail without fallback. Newly created directories are private
on Unix; existing project permissions are preserved. Codex resolves project configuration in
this selected workspace while its normal HOME/CODEX_HOME skills, plugins and account remain
available. Craft does not copy the profile or rewrite its configuration. Settings displays
both the selected workspace and effective profile directory for verification.
Existing app conversations remain in Craft's app-specific history.

`CRAFT_CODEX_BINARY` is an exact executable override. Otherwise discovery tries PATH first,
then the reviewed Codex.app/ChatGPT.app CLI bundle locations in macOS application directories.
An empty/missing explicit override fails rather than selecting another runtime. The active
subscription connection requires a ChatGPT account in that selected profile; an API-key account
is not silently selected instead. The sign-in action reuses an existing signed-in profile.
Signing in when needed uses Codex's normal profile, rather than a separate Craft account.

Codex loads its own base instructions, skills, plugins, MCP servers and configured permissions.
Craft adds developer instructions and the registered document tools, without a built-in-tool
allowlist, forced sandbox, disabled-feature list or different approval policy. Explicit `$skill`
references are matched to enabled `skills/list` entries and sent as native skill input items.
The selected model must match the thread/start result; model/provider fallback is not accepted.
Catalog discovery alone does not prove inference access.

Native command/file approvals, permission requests, user questions and MCP form/URL elicitations
appear inside the assistant pane. Nothing is approved or submitted merely by rendering a request.
Responses are correlated to one current request, bounded, validated against offered decisions or
requested grants, and sent once. Cancel/Stop remains available. Unknown request types fail with
an actionable error. MCP forms outside the supported scalar field set use explicit JSON input.
Secret answers are transient and action Debug output omits payloads. URLs open only on an explicit
user action. No approval rewrites user configuration unless the user explicitly chooses a proposed
persistent command/network rule.

`CodexProvider::with_interaction` supplies the native interactive callback for non-GUI clients.
The helper exposes private-pipe commands `interaction_pending` (null or current id/method/params)
and `respond` (`{id,response}`), using the same validated path as the UI. Callers must not log
request parameters, answers or authentication URLs. Bridge/settings/history formats remain1;
approval rendering requires the new helper, not a host rebuild.

Registered Craft document calls retain exact target/turn/tool identity checks, canonical result
continuation, host file/root permissions and app Undo. Native shell/plugin changes outside the
app engine follow Codex's policy and are not covered by app Undo. User content remains data, not
instructions. No operation or uncertain result is retried automatically. Each user turn owns
fresh callback state and an ephemeral native thread; Craft retains its own canonical transcript.

Human response waits remain cancelable and restart the inference receive deadline after the
response. Stop/drop first sends native `turn/interrupt`, then closes stdin for Codex's tool/MCP
cleanup; only an unresponsive server is force-stopped after the existing configured connect
timeout. Runtime request/context/response limits remain in the shared RuntimeConfig.

The public protocol reference is [Codex App Server](https://learn.chatgpt.com/docs/app-server).
Implementation was checked against the installed0.166.0-alpha.6 experimental JSON schema.
The dated verification records below describe earlier isolated builds unless noted otherwise;
the former containment probes are not the normal-profile permission contract.

## Verification record

- Shared core and native focused tests and clippy were run by the runtime workstream; see its
  handoff for final counts after integration.
- `cargo test -p craft-ai-native codex::tests::installed_runtime_denies_reads_writes_and_network -- --ignored`
  passed on macOS with installed Codex 0.165.0 on 2026-10-28. It checked strict effective policy,
  denied reading the synthetic sentinel, denied writing in the worker folder, and denied a loopback
  connection with successful outside-sandbox and executable-availability controls. No real account,
  credentials, document, or conversation was read.
- Platform credential acceptance uses
  `cargo test -p craft-ai-native secrets::tests::platform_store_round_trip -- --ignored`.
  This passed on macOS on 2026-10-28: one synthetic item was written, read, and deleted; no existing credentials were accessed.
- Unit regressions cover shared-store concurrent updates, newer-version refusal, app-separated
  histories, interrupted conversations, storage traversal refusal, registered tool/thread/turn
  validation, tagged tool protocol, weaker-policy refusal, exact executable overrides, synthetic
  callback/continuation failure, and compatibility-cache invalidation.

Remaining live gates: Codex sign-in and model inference, a real dynamic model callback through
WordCraft MCP, model-driven containment adversarial checks, hosted/local model requests, and
Windows/Linux/BSD sandbox and credential-store acceptance. The live loopback network probe is
verified on macOS; it is not evidence of all platform/network combinations. Mock tests
and the synthetic macOS sandbox probe do not complete those gates.

## Gate follow-up, 2026-10-28

The initial implementation ran only the filesystem probe before inference. It now requires the
network and selected-model callback/continuation checks above before caller context can reach
Codex. No real model inference or sign-in was performed during this change. Focused regression
tests passed; the extended installed-runtime macOS containment test passed. The final native
package run passed 34 ordinary tests with two platform tests ignored by default
(`/private/tmp/craft-assistant-codex-gate-tests.log`). The explicitly run containment test passed
(`/private/tmp/craft-assistant-codex-gate-live.log`). Initial native Clippy passed; a later run
identified only a concurrent runtime-test lint, subsequently fixed before passing full shared gates.
The implementation ledger owns final shared gate evidence.

### Home-discovery hardening, 2026-10-28

A temporary synthetic skill proved that Codex 0.165.0 discovers `HOME/.agents/skills` when
`CODEX_HOME` alone is isolated. With all three home variables pointed at the Craft profile,
`skills/list` no longer discovered that external synthetic skill. Only temporary test content
was used; no user skills, accounts, credentials or documents were inspected. The child command
now enforces that isolation, with a unit regression checking its environment.

Verification after home isolation and the assistant document-label localization fix:
`cargo fmt --check`, full `cargo clippy --workspace --all-targets -- -D warnings`, and
unfiltered `cargo test --workspace` passed (1,831 passed, 28 ignored). Core and UI
`cargo check --target wasm32-unknown-unknown -p craft-ai-core -p craft-ui` passed.
Nine focused Codex tests passed; the ignored installed-runtime read/write/network probe
passed separately with the new isolated environment. Logs use the
`/private/tmp/craft-assistant-home-` prefix; see the implementation ledger for exact paths.

## Codex tool continuation repair — 2026-10-28

A live synthetic WordCraft request exposed a transport defect after sign-in succeeded: chat
delegated correctly, but control repeated eleven successful document reads and reached its round
limit without editing. The adapter previously killed each worker at its first tool call and sent
tool records as JSON user text to a new thread. The old compatibility probe forced its second
answer with another user instruction and no tools, so it did not test normal continuation.

The adapter now replies to dynamic tool requests in the same ephemeral turn, with exact canonical
prefix/configuration matching, bounded retained workers, cancellation and no result replay.
Synthetic subprocess regression tests cover read/edit/read-back, nested chat/control resumption,
separate JSON-RPC IDs and call IDs, seven mismatch classes, refusal, disconnect, duplicate IDs,
blocked pipe cancellation, provider drop and the two-exchange limit.

Focused verification passed: `cargo test -p craft-ai-native` (45 passed, 2 pre-existing live tests
ignored), `cargo clippy -p craft-ai-native --all-targets -- -D warnings`, and native package format
checking. Evidence: `/private/tmp/craft-codex-continuation-native-all-tests.log` and
`/private/tmp/craft-codex-continuation-clippy.log`. Initial strict Clippy identified one denied
macro in a new test; it was removed before the passing run. Combined shared-library gates and
repaired-preview live Codex editing remain pending at this checkpoint. No credentials were read
or copied and no live inference was run for this repair's focused verification.

## Initial document context

Hosts can supply a read-only `ToolHost::initial_read` for delegated application tasks. The
runtime executes it through the same catalog validation, cancellation, checkpoint and activity
path as a model-requested tool before the first control-model response. The result is untrusted
document data in a normal tool message. Hosts retain target/file authorization. General chat
does not invoke the hook. This avoids accepting an empty-activity denial of document access
from a model that never requested inspection; it does not guarantee the model can finish an edit.

## Legacy identical model selections

For consumers of the compatibility two-role request, when General chat and App control resolve to the identical connection and model, the runtime
uses one bounded tool loop instead of delegating to the same model and then summarizing back
through it. It exposes the host's scoped tool catalog directly. General questions do not
automatically read document data; actual tool calls still pass the host's document/file guards.
The shared dispatch path retains cancellation, whole-batch validation, durable outcome
checkpoints, response/context bounds and no-replay handling. Different selections retain the
separate control_app route and host-initialized document context. No provider fallback occurs.

## Unified model checkpoint — 2026-10-29

The shared pane now presents one **Assistant model** selection for questions and document work,
with one `Model:` route in its header. Existing connection management supports the OpenAI
subscription (Codex) connection, local Ollama and compatible API connections, including
OpenRouter. Model discovery, tool verification, credential references, per-app overrides and
suite defaults retain their existing paths; no model or endpoint is silently substituted.

`resolve_choice(store, app)` resolves the canonical `AssistantRole::Chat` / `assistant.chat`
binding. The saved legacy Control binding remains intact but is not a fallback or a second
setting in the unified pane. The older `resolve_choices` and two-role `TurnRequest` remain
compatibility APIs. A unified consumer supplies the one resolved choice for both request fields
to select the existing direct tool loop. Host authorization, cancellation, checkpointing,
whole-batch validation and no-replay safeguards still apply. Document data is supplied through
scoped tool calls when needed; unrelated general prompts do not automatically read it.

Current focused verification passed: native tests **56 passed / 5 ignored**, UI tests
**465 passed / 2 ignored**, native/UI all-target Clippy with warnings denied, and core/UI WASM
checking. Regressions cover a single visible selector/route, preservation of legacy bindings,
canonical Chat resolution without Control fallback, isolated callback state per fork and a fresh
account check despite cached runtime proof. A live synthetic Codex transport probe measured
**14.748 seconds cold / 8.531 seconds warm**. These are transport-probe observations, not a
document-edit benchmark or evidence of panel/external-client performance parity. Consumer
edit/save/reopen/Undo, same-fixture model comparisons and packaged delivery remain separate
gates recorded in the WordCraft consumer ledger.


## Historical helper0.1.3 GUI-launched Codex discovery — 2026-11-04

Helper0.1.3 includes PATH-first installed-bundle discovery on macOS. The change selects the
same Codex CLI runtime; it does not select another provider, inherit its desktop account, copy
history/configuration, or change Craft's isolated Codex home. The existing compatibility and
network-containment checks still run before document content is sent.

Deterministic candidate tests cover absent PATH candidates, installed-bundle discovery, PATH
precedence and exact overrides. A separate explicitly run installed-bundle smoke starts only
`--version` with `/usr/bin:/bin` as the child PATH; authentication and inference are separate checks.

Source verification: 38 native-provider unit tests, 6 HTTP tests and 24 synthetic runtime
integration tests passed; 6 manual/live checks remained ignored. All 28 helper runtime tests
and 4 process tests passed. Scoped all-target Clippy with warnings denied, formatting and
whitespace checks passed. Removing installed-bundle discovery makes its deterministic test
fail; restoring it passes. The explicit local installed-bundle smoke resolved the ChatGPT CLI
and printed `codex-cli 0.166.0-alpha.6`. This proves process discovery/startup only; authenticated
provider compatibility, containment and inference remain separate from this version check.

Helper0.1.6 follows Codex's normal configuration-loading semantics without forcing
`--strict-config`. The strict override rejected a legacy skills configuration field accepted
by the same installed Codex executable in normal mode, disconnecting before a native turn.
No profile or provider fallback is introduced; the original normal profile is still used.
A command-builder regression checks the exact app-server argument list. Other startup/runtime
errors remain reported to the user without replay; stderr stays private and is not logged.
