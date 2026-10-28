# Portable assistant implementation ledger

Last updated: 2026-10-28. Delivery state: **Verified shared implementation**; provider/platform live acceptance remains open.
Initial implementation `566be16052de3cbe04fa132a88c24db0b7c7f756` was published with user approval to
`andykumeda/craft-libs`, branch `codex/portable-assistant`, and its remote SHA was verified. This
follow-up commit adds verified Codex home isolation and document-label localization. The consumer
ledger records the final published revision and WordCraft pin. No app release has occurred.

The approved first integration is WordCraft desktop. Other Craft app integrations and browser
inference are deferred. Shared code must not import an application engine or document model.

| Workstream / owner | Acceptance and modules | State | Verification / outstanding work |
| --- | --- | --- | --- |
| Contracts / runtime agent | `ai-core::assistant`: messages, identified calls/results, target token, role declarations, configuration | Verified | AI-core tests: 221 passed; focused Clippy passed |
| Native HTTP and routing / runtime agent | Two independent model roles, automatic delegation, streaming, cancellation, bounds, no fallback/replay | Verified | Final native tests: 36 passed, 2 platform tests ignored by default and passed separately on macOS; two synthetic app hosts, loopback HTTP and recovery/context review regressions covered; full Clippy passed |
| Native services / services agent | Locked store, credential references, history, isolated Codex app-server adapter | Verified (automated and local OS); live callback pending | Codex inference now requires runtime read/write/network denial and selected-model synthetic callback/continuation before caller context; expanded macOS sandbox probe passed; macOS synthetic keychain round-trip passed; real inference/platform gates below |
| Reusable widgets / widget agent | Transcript, composer, settings, model picker, actions, activity, Stop | Verified | Seven assistant UI tests passed within 462 passing craft-ui tests; ignored screenshot capture passed separately and PNG inspected by widget agent |
| Full shared gates / runtime agent | Formatting, workspace Clippy/tests, core/UI WASM | Verified | Final formatting, full all-targets Clippy, unfiltered workspace tests and core/UI WASM passed. Workspace total after home-isolation hardening: 1,831 passed, 28 ignored; no tests filtered out. |
| WordCraft integration / primary agent | MCP bridge, native services, docking/commands/settings, document/file guards | In progress | Separate WordCraft worktree ledger owns app evidence |
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

Real hosted/local inference, Codex account sign-in and complete containment, Windows/Linux/BSD
credential stores and WordCraft end-to-end acceptance are distinct gates. Mock, loopback and
headless tests must not be reported as these checks. No private document, real credential value
or existing conversation has been read for development.

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
- No real sign-in, selected-model callback or WordCraft live MCP turn was performed. The gate is
  executable after the user signs in; it cannot be bypassed with a persisted flag.

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

## Revision history

| Date | Change |
| --- | --- |
| 2026-10-28 | Runtime recovery/security review fixes and final full shared gates passed |
| 2026-10-28 | Initial implementation checkpoint and automated test evidence |
| 2026-10-28 | Isolate Codex global home-based discovery; rerun full gates with localized assistant document label |
