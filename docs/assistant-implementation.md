# Portable assistant implementation ledger

Last updated: 2026-10-28. Delivery state: **Verified shared fixes and installed WordCraft Codex
and mixed Codex/Ollama editing/read-back/Undo**. The
[WordCraft consumer ledger](/Users/andy/Dev/wordcraft/docs/assistant-implementation.md)
is authoritative for installed application, account, model-routing and live document evidence.
Initial implementation `566be16052de3cbe04fa132a88c24db0b7c7f756` was published with user approval to
`andykumeda/craft-libs`, branch `codex/portable-assistant`, and its remote SHA was verified. The
previous follow-up `a8d4c0a178aff4c31d27e05bf87a4e93915bc831` added Codex home isolation and
document-label localization. Published shared revision
`d4641d3663819aeb20cab5ccc1623292c5f4e30e` repairs native Codex tool continuation and long-title
assistant layout. The consumer ledger records publication, its pinned revision and local preview
installation; no public app release has occurred.

The approved first integration is WordCraft desktop. Other Craft app integrations and browser
inference are deferred. Shared code must not import an application engine or document model.

| Workstream / owner | Acceptance and modules | State | Verification / outstanding work |
| --- | --- | --- | --- |
| Contracts / runtime agent | `ai-core::assistant`: messages, identified calls/results, target token, role declarations, configuration | Verified | AI-core tests: 221 passed; focused Clippy passed |
| Native HTTP and routing / runtime agent | Two independent model roles, automatic delegation, streaming, cancellation, bounds, no fallback/replay | Verified | Final native tests: 45 passed, 2 platform tests ignored by default and passed separately on macOS before this follow-up; two synthetic app hosts, loopback HTTP and recovery/context review regressions covered; full Clippy passed |
| Native services / services agent | Locked store, credential references, history, isolated Codex app-server adapter | Verified (automated, macOS OS and WordCraft Codex live callback) | Codex inference now requires runtime read/write/network denial and selected-model synthetic callback/continuation before caller context; expanded macOS sandbox probe passed; macOS synthetic keychain round-trip passed; real inference/platform gates below |
| Reusable widgets / widget agent | Transcript, composer, settings, model picker, actions, activity, Stop | Verified | Eight assistant UI tests passed within 463 passing craft-ui tests; ignored long-title capture passed separately; idle/running PNGs inspected |
| Full shared gates / runtime agent | Formatting, workspace Clippy/tests, core/UI WASM | Verified | Final formatting, full all-targets Clippy, unfiltered workspace tests and core/UI WASM passed. Workspace total after continuation/layout repair: 1,841 passed, 29 ignored; no tests filtered out. |
| WordCraft integration / primary agent | MCP bridge, native services, docking/commands/settings, document/file guards | Verified for installed Codex and mixed-provider editing | Authoritative WordCraft consumer ledger records source `1692eef9`, shared pin `d4641d3`, exact edit/read-back and one normal Undo |
| Documentation and commits / primary agent | Current docs, porting guide, focused shared commit and consumer pin | In progress | Initial A1 source published to the approved fork; verified follow-up included in this commit; final consumer pin tracked in WordCraft |

## Checkpoint and evidence

- Base: `92d392cff132eaf53e13afff77f38b5c11b73aa0`; branch `codex/portable-assistant`.
- Shared checkout: `/Users/andy/Dev/craft-libs`.
- Runtime agent build cache `/private/tmp/craft-assistant-runtime-target` was removed after
  both initial validation and final review gates to recover disk space; logs below remain.
- Full Clippy log: `/private/tmp/craft-assistant-shared-clippy.log`.
- Final unfiltered workspace test log: `/private/tmp/craft-assistant-shared-test.log`.
  All 1,830 tests/doc-tests passed; 28 are ignored by default. This includes 221 AI-core tests,
  35 native tests and 462 UI tests; no tests were filtered out.
- The original invalid-byte font fixture attempted a filename APFS rejects with OS error 92.
  Its replacement directly supplies the same invalid `PathBuf` to the real include-bytes
  source-generation boundary. No filesystem errors are suppressed and its regression passes.
- The settings Stop test now advances a bounded number of busy frames because its spinner
  intentionally repaints continuously; its idle phases still use the settling harness.
- WASM log: `/private/tmp/craft-assistant-shared-wasm.log` (core and UI passed).
- Formatting log: `/private/tmp/craft-assistant-shared-fmt.log` (passed).
- Focused review logs: `/private/tmp/craft-assistant-review-tests.log` and
  `/private/tmp/craft-assistant-review-clippy.log` (passed before final shared gates).
- The fake HTTP server now explicitly clears inherited nonblocking mode on accepted sockets;
  the final rerun passed after exposing and fixing that macOS test-fixture race.
- Rust 1.97.1; builds use two jobs, no debug info and no incremental cache for disk control.
- Baseline manifest repair: workspace `craft-keymap` now declares `default-features = false`,
  matching `craft-menu`'s inherited declaration. `craft-keymap` has no default feature, so this
  is behavior-neutral and permits Cargo 1.97 to parse the workspace.
- Fake HTTP tests bind and retain OS-assigned `127.0.0.1:0` sockets. They close after each test;
  no fixed service, proxy or persistent listener is added. The local service inventory and live
  listeners were inspected first.
- The instructed `craftrules` repository was absent locally and returned 404 through the
  connected account. Existing repository instructions remain applicable.

## Runtime review fixes

- Recovery checkpoints synchronously journal paired delegation results before host dispatch and
  after outcomes. Completed, refused and uncertain operations remain visible after interruption;
  saving a checkpoint can cancel the turn before an edit. Success history retains control activity.
- The entire tool-call batch is validated before dispatch. Repeated identities and malformed later
  calls cannot execute a valid prefix; uncertain channel outcomes stop without retries.
- Saved history rejects system injection and unmatched/misplaced tool messages. Tool catalogs count
  toward context limits. Oversized returned data records the call identity and completed outcome,
  omits the body, and stops. Disabled connections fail without fallback.
- HTTP response text, tool arguments and model catalogs redact known credentials; streaming
  redaction retains partial key suffixes across chunks. Empty/oversized stored credentials fail.
- Regressions cover interruption after completion, uncertain outcomes, persistence cancellation,
  duplicate batches, context/catalog bounds, malformed histories and fragment-safe redaction.

## Limits and release gates

Live provider use, Codex account sign-in and containment, platform credential stores and WordCraft
end-to-end acceptance are distinct gates. The WordCraft consumer ledger records passed macOS
Codex sign-in, callback/continuation, MCP editing/read-back/Undo, earlier local-model acceptance,
and mixed Codex-chat/Ollama-control editing/read-back/Undo. Hosted compatible endpoints,
broader adversarial live scenarios and Windows/Linux/BSD credential stores remain separate open gates. Mock, loopback and headless tests must not be reported
as live checks. Development and acceptance use synthetic documents and task-created conversations;
no real credential value or unrelated private document/conversation was read.

## Codex gate follow-up

- `CRAFT_CODEX_BINARY` selects an executable explicitly; empty/missing values fail without fallback.
- Before caller context is sent, `complete` enforces the effective isolated policy, synthetic denied
  read/write checks, positive-controlled loopback network denial, isolated sign-in and selected-model
  callback/result continuation. Successful compatibility is cached only in memory for the resolved
  executable metadata, exact model and runtime limits. Each worker still validates effective policy.
- Actual macOS Codex 0.165.0 read/write/network containment probe passed with no account, document
  or credential reads. Log: `/private/tmp/craft-assistant-codex-gate-live.log`.
- Native package tests passed 34 ordinary tests and ignored two platform tests by default. Log:
  `/private/tmp/craft-assistant-codex-gate-tests.log`. Later shared-gate runs supersede these counts
  as the integration workstreams finish. Source ownership and caller APIs remain unchanged.
- At that earlier checkpoint no real sign-in, selected-model callback or WordCraft live MCP turn
  had been performed. The later consumer acceptance below supersedes that live-gate status. The
  gate cannot be bypassed with a persisted flag.

## Home-isolation hardening verification

State: **Verified**, included in this follow-up commit; final consumer repin tracked in WordCraft. Base commit: `566be16`.
Codex now receives the Craft profile for `HOME` and `USERPROFILE` as well as `CODEX_HOME`.
A disposable synthetic `.agents` skill established the original discovery path in installed
Codex 0.165.0 and its absence when all home variables are isolated. No real user skill content
was read. An environment regression verifies the command's home and config variables.

The same gate run includes the shared assistant's localized `Document: {name}` label.

- `cargo fmt --check`: passed (`/private/tmp/craft-assistant-home-fmt.log`).
- `cargo clippy --workspace --all-targets -- -D warnings`: passed
  (`/private/tmp/craft-assistant-home-shared-clippy.log`).
- `cargo test --workspace`: passed, unfiltered, **1,831 passed / 28 ignored**
  (`/private/tmp/craft-assistant-home-shared-tests.log`).
- `cargo check --target wasm32-unknown-unknown -p craft-ai-core -p craft-ui`: passed
  (`/private/tmp/craft-assistant-home-wasm.log`).
- Focused Codex tests: nine passed, one installed-runtime test ignored by default
  (`/private/tmp/craft-assistant-home-tests.log`).
- The ignored installed-runtime filesystem/network containment test passed separately on macOS
  with all home variables isolated (`/private/tmp/craft-assistant-home-live.log`).

The disposable `/private/tmp/craft-assistant-home-target` build cache was removed after gates.
These checks do not imply real Codex sign-in or model inference. WordCraft's separate live-model
acceptance and temporary local-service lifecycle remain in the consumer task ledger.

## Long-history-title layout follow-up

State: **Verified**, including the full workspace and WASM rerun below. No installed app was
changed by this widget work; the consumer owns rebuilding and installing the corrected preview.

A long active chat title could consume the entire combined header row. The History label then
wrapped vertically and expanded the toolbar, pushing the composer and Stop below the pane. The
regression reproduced a 105-pixel-high History control at a 280-pixel pane width. The header now
uses separate action and history rows; the visible History label precedes a dropdown bounded by
the remaining row width and supplies its accessible name. Buttons retain their existing enabled
states, and no host/runtime API changed.

- The geometry regression covers widths 280, 420 and 700, heights 480 and 800, and both idle and
  running states. It checks one-line History controls, compact header geometry, and in-bounds
  composer and Stop controls. The original header failed; the fixed header passes all 12 cases.
- Focused assistant tests: **8 passed, 2 manual captures ignored by default**
  (`/private/tmp/craft-assistant-layout-tests.log`).
- The long-title capture test passed separately, and all four idle/running screenshots were
  inspected: `/private/tmp/craft-assistant-layout-{280,420}-{idle,busy}.png`
  (`/private/tmp/craft-assistant-layout-screenshots.log`). These are synthetic shared-widget
  captures, separate from the consumer app's live-provider acceptance.
- `cargo clippy -p craft-ui --lib --tests -- -D warnings`: passed
  (`/private/tmp/craft-assistant-layout-clippy.log`). Scoped rustfmt and diff checks passed
  (`/private/tmp/craft-assistant-layout-fmt.log`).
- The independent cache `/private/tmp/craft-assistant-layout-target` is retained temporarily for
  the parent integration's full gates. No source changed after these focused checks.

## Codex continuation and combined verification follow-up

State: **Verified shared fixes and installed Codex plus mixed-provider edit/read-back/Undo.** The user completed
Craft-managed Codex sign-in in WordCraft and selected `gpt-6.1-sol` independently for both roles.
The first synthetic native edit attempt made 11 successful document reads and no edits, then hit
its round limit. Its original text remained unchanged. WordCraft owns the synthetic evidence and
all account/configuration operations; no credential values were read or copied into this repository.

The previous adapter closed its worker at each tool callback and supplied tool results as quoted
JSON history to a new thread. The new adapter keeps one ephemeral exchange per active role and
replies to the original JSON-RPC request with the actual canonical tool result. Exact history,
call identity, connection/model, catalog, runtime limits and executable identity must match.
Cancellation, expiry, disconnect and final provider drop terminate workers; failed or uncertain
writes never replay. The synthetic compatibility check now keeps tools available and continues
from the actual result without appending another user instruction. Public rustdoc and services
help require provider ownership per active turn or standalone probe, including cleanup on every
exit; WordCraft and its corrected standalone live-test probes own providers this way.

Combined final gates on this source all passed:

- `cargo fmt --check`: `/private/tmp/craft-assistant-continuation-shared-fmt.log`.
- `cargo clippy --workspace --all-targets -- -D warnings`:
  `/private/tmp/craft-assistant-continuation-shared-clippy.log`.
- `cargo test --workspace`: **1,841 passed / 29 ignored**, unfiltered, no failures;
  `/private/tmp/craft-assistant-continuation-shared-tests.log`.
- `cargo check --target wasm32-unknown-unknown -p craft-ai-core -p craft-ui`:
  `/private/tmp/craft-assistant-continuation-shared-wasm.log`.
- Focused native tests: **45 passed / 2 ignored**, including actual pipe response/continuation,
  nested chat/control, mismatches, duplicate identities, cancellation/drop, disconnect and blocked
  write regressions; `/private/tmp/craft-codex-continuation-native-all-tests.log`.
- Independent review found no remaining WordCraft continuation defect; the explicit portable
  provider-lifetime contract above is required for consumers retaining providers.

No new dependency, application model dependency or public API was added. Task-owned build caches were removed after the final consumer and shared delivery checks.
The corrected installed WordCraft Codex edit/read-back/Undo checkpoint passed as recorded below.
The user-requested Codex-chat/Ollama-control routing test also passed. Hosted API, broader
adversarial live scenarios and other-platform acceptance remain separately gated; other Craft
integrations and browser inference remain deferred.

## Installed WordCraft live acceptance and delivery verification

State: **Verified for Codex and mixed-provider editing/read-back/Undo.** The consumer's
[authoritative live ledger](/Users/andy/Dev/wordcraft/docs/assistant-implementation.md)
records installed WordCraft source `1692eef9cc87ced4d2a26e90f0b203e357abacce` with shared
`d4641d3663819aeb20cab5ccc1623292c5f4e30e` on the Mini. Both independently configured roles used
Codex `gpt-6.1-sol`; the synthetic task completed in about 30 seconds with one `control_app`
delegation and six successful MCP calls: `get_text`, `inspect_document`, `select_text`, `type_text`,
`get_text`, `inspect_document`. The document changed only from 9:00 AM to 10:00 AM, read-back
matched exactly, and one normal Undo restored the full baseline. The native completion screenshot
was inspected. This is installed consumer evidence, not a shared mock/test claim or public release.

Synthetic consumer artifacts: `/private/tmp/wordcraft-routing-_u55a7zx/codex-status.json`,
`codex-conversation.json`, `codex-verification.json` and `codex-complete.png`. The shared services
agent inspected the verification artifact and consumer ledger; the consumer retains responsibility
for account interaction, live inference, screenshot review, service lifecycle and final acceptance.
The shared ledger does not copy credentials or private account details.

The mixed-provider installed test passed in **256.04 seconds**. Captured native status transitions
showed Codex/`gpt-6.1-sol` for chat, Ollama/`gemma4:e4b` for control, then Codex for the final chat
response. One `control_app` delegation produced four successful MCP calls: `get_text`,
`select_text`, `type_text`, `get_text`. Independent document inspection confirmed the exact
9:00 AM to 10:00 AM change and one normal Undo restored the baseline. The owned Ollama endpoint
received five successful HTTP 200 `POST /v1/chat/completions` requests after the test boundary;
`/api/ps` recorded the selected local model and digest. No model was downloaded.

Mixed consumer evidence in the same synthetic directory: `mixed-status.json`,
`mixed-conversation.json`, `mixed-verification.json`, `mixed-ollama-active.json`,
`mixed-ollama-requests.log` and `mixed-complete.png`. The verification artifact confirms the two
independent routes, one delegation and four completed MCP operations without errors. Saved chat
records alone do not persist provider/model provenance; the consumer's captured native routing
transitions and local transport/model evidence substantiate this mixed-provider result. Broader
hosted-provider, adversarial and other-platform live acceptance remain open.

The required full shared delivery rerun is **Verified**, using
`/private/tmp/craft-assistant-layout-target` with two build jobs, no debug info and no incremental
cache. All four commands exited successfully:

- `cargo fmt --check`: `/private/tmp/craft-assistant-delivery-fmt.log`.
- `cargo clippy --workspace --all-targets -- -D warnings`:
  `/private/tmp/craft-assistant-delivery-clippy.log`.
- `cargo test --workspace`: **1,841 passed / 29 ignored / 0 failed / 0 filtered out** across
  120 test summaries; `/private/tmp/craft-assistant-delivery-tests.log`.
- `cargo check --target wasm32-unknown-unknown -p craft-ai-core -p craft-ui`:
  `/private/tmp/craft-assistant-delivery-wasm.log`.

`git diff --check` also passed. The local service inventory and current socket owners were
inspected before the native transport tests, which retained their own OS-assigned loopback
sockets. Existing consumer Ollama and preview listeners were not changed. No code or dependency
changes are part of this ledger update. The primary agent commits and publishes this evidence
separately; the installed consumer remains pinned to reviewed runtime revision `d4641d3`.

## Revision history

| Date | Change |
| --- | --- |
| 2026-10-28 | Runtime recovery/security review fixes and final full shared gates passed |
| 2026-10-28 | Initial implementation checkpoint and automated test evidence |
| 2026-10-28 | Isolate Codex global home-based discovery; rerun full gates with localized assistant document label |
| 2026-10-28 | Keep long saved-chat titles within a compact, accessible assistant header; add idle/running geometry and screenshot evidence |
| 2026-10-28 | Repair native Codex result continuation, retain canonical Craft history and pass all shared gates with long-title layout fixes |
| 2026-10-28 | Record installed WordCraft Codex and mixed Codex/Ollama editing/read-back/Undo acceptance and rerun shared delivery gates |

## 2026-10-29: Initial delegated document read

Status: **Verified in the native runtime; consumer live acceptance pending**. An owner report
contained an App control result with empty activity and a denial of document access. The runtime
previously allowed a delegated control response without any read. Hosts can now supply an
initial read-only tool, dispatched through ordinary catalog/target checks, checkpointing,
cancellation and activity before the first control response. WordCraft opts into inspection.
General chat still does not read the document. This does not establish that either model will
follow every instruction or complete every edit.

`app_control_has_document_context_before_its_first_answer` failed on the original path, then
passed after the fix. Native suite: **49 passed / 2 ignored / 0 failed**; all 17 orchestration
tests passed. Native all-target Clippy with warnings denied, workspace format and diff checks
passed. Logs: `/private/tmp/craft-initial-context-{red,green,tests,clippy}.log`. No dependencies,
model bindings, credentials or global context settings changed. Consumer A/B acceptance and
packaged delivery remain pending; privacy separation is an owner's soft preference, not an
implemented local-only document route.

### Codex initial-context start correction

The consumer's real pre-deployment model test caught a gap missed by the mock regression:
host-initialized context ended in a Tool message, which Codex correctly treated as a callback
continuation despite having no corresponding provider callback. That candidate was not deployed.
After the initial host read, the runtime now repeats the authorized task as a User message to
start control inference. Genuine subsequent provider tool results still remain Tool messages
and retain exact continuation validation. A deterministic regression first failed on the old
shape and now checks both the initial User shape and later Tool continuation.

The opt-in `live_codex_host_context_starts_and_continues` test passed with synthetic read-only
host data using the configured Codex account: **20.310 seconds, two reads, control inference
used, completed normally**. No application document or chat history was read. This is transport
acceptance, not full document-edit/save/Undo acceptance. Log:
`/private/tmp/craft-host-context-codex-live.log`. Native tests passed **49 / 3 ignored / 0 failed**;
all-target native Clippy, workspace format and diff checks passed. Final logs:
`/private/tmp/craft-host-context-start-final-{tests,clippy}.log`. Owner directed Codex-first
consumer acceptance before proceeding to Qwen; no Qwen routing changes are included.
