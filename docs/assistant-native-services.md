# Assistant native services

Status: implemented, integration in progress. Last checked: 2026-10-28.

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

## Codex adapter

Route a connection whose provider is `codex` to `CodexProvider::new(CodexOptions::in_shared_root(root))`.
`CodexOptions::in_shared_root` reads `CRAFT_CODEX_BINARY` when set; otherwise it resolves `codex`
from `PATH`. Set an absolute path for installations outside the application's `PATH`. An empty or
missing selected executable fails explicitly without substituting another runtime. `CodexOptions`
also permits apps to supply an executable path directly and an absolute, dedicated home. It refuses
the user's existing default or active Codex home. `CODEX_HOME`, `HOME`, and `USERPROFILE` all
point to that dedicated home: Codex discovers global `.agents` skills through `HOME` independently
of `CODEX_HOME`. The child inherits platform plumbing only, not API keys, alternate user-config
roots, or proxy/provider configuration. Existing coding-session configuration is not changed or copied.

- `signed_in(config, cancellation)` reports only whether the isolated account is a ChatGPT account.
- `start_login` returns a `LoginSession` with an HTTPS authorization URL and one-time code. Keep
  this on a worker and call `poll`; dropping it stops the sign-in helper. Never persist or log the
  one-time code. Login happens only after the user's sign-in action.
- `models` pages through the runtime's model catalog. Catalog discovery alone does not enable
  inference. The provider implements the shared `ChatProvider::complete` interface for streamed
  text and identified tool requests. Before sending any caller-supplied context, `complete` calls
  `verify_compatibility(connection, model, config, cancellation)` with synthetic data only.
- Each model round uses a new ephemeral thread and bounded canonical transcript. A dynamic tool
  request must match its thread, turn, and registered tool name. The worker exits before the host
  executes the call; the next round starts with the recorded tool result. This avoids depending
  on the unsupported `thread/resume.history` interface. It trades some startup overhead for a
  single app-owned transcript and no pending calls left inside another process.
- Cancellation checks interrupt waiting and kill the owned process. Disconnects, rejected requests,
  and uncertain outcomes are reported without automatic retry.

The adapter disables shell execution, local image viewing, browser/computer tools, external
apps/plugins, hooks, multi-agent work, memories, image generation, and other unrelated features.
It uses the named `craft-assistant` permission profile: root reads denied, minimal system paths
readable, only the empty working directory readable, no writes, network disabled, and approvals
never. The permission table is passed as one TOML CLI override; dotted CLI keys containing quoted
filesystem names are not equivalent and were rejected by Codex 0.165.0.

Before inference, `verify_compatibility` requires all of the following:

1. `verify_isolation` checks effective configuration and launches the real Codex OS sandbox
   against a synthetic denied-read sentinel and a forbidden write in the empty folder.
2. The same sandbox must deny a connection to a retained, OS-assigned `127.0.0.1:0` listener.
   A working `curl` executable is required. The identical request must first succeed outside
   the sandbox, and `curl --version` must succeed inside it. The blocked request must return
   connection failure (curl status 7) without reaching the listener. Startup failures do not
   count as network-denial proof. All child processes have bounded, cancellable waits.
3. The isolated account must be signed in. The selected model must call exactly one synthetic
   dynamic tool with the supplied challenge, then correctly consume its recorded result in a
   new ephemeral thread. The test has no host adapter, document, file grants or edit command.
   Only synthetic messages are sent, with the configured request/context/response limits.

Failures leave inference disabled with an explicit error. No flag or persisted acceptance file
bypasses these checks. Successful checks are cached in memory for the provider instance, resolved
executable path/size/modification time, exact model, and runtime limits. Clones share that cache;
new instances or changed identities recheck. Effective policy is still checked on every worker.
Login and catalog discovery remain available so an unverified connection can be configured.

The public app-server API has no dynamic-tools-only allowlist: residual built-in tools are
constrained by the sandbox and unexpected activity is rejected. This runtime check validates
model callback transport and canonical result continuation; a real callback through each app's
host adapter remains a separate integration acceptance test. Network probing uses only a temporary
loopback listener, with no fixed port, proxy, persistent service or external request. The listener
is retained during the checks and closes afterward; a fresh run allocates a fresh port.

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
