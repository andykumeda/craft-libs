# Independently updated assistant

Status: implementation committed on task branches after focused automated checks, 2026-11-02. No installed runtime or release has been verified.

The assistant remains embedded in PDFCraft, WordCraft and GridCraft. A shared executable owns its interface and native services; each app retains document authorization, editing and Undo. Private process pipes carry a versioned protocol. Model/provider selection and authentication use the existing suite-wide store and credential namespace. App overrides remain optional; chats and file grants remain app-separated.

## Task ledger

| Workstream / owner | Acceptance | Modules | Dependencies | State | Evidence / release |
| --- | --- | --- | --- | --- | --- |
| Embedding / transport agent | Input, rendering, IME and accessibility cross private pipes without exposing egui layouts | assistant-bridge | Protocol v1 | Verified | Bridge: 14 synthetic tests; native acceptance pending |
| Assistant services / primary | Existing providers, settings and interrupted history work outside the host app | assistant | Bridge | Verified | Runtime/controller/proxy tests and four real helper-process tests; final counts below |
| PDF integration / primary | Existing authorized PDF tool host drives the embedded helper | pdfcraft assistant adapter | Bridge/services | Verified | Production UI check + Clippy passed; live native acceptance pending; not deployed |
| Word/Grid integration / primary | Same shared helper, isolated documents/history, existing automation preserved | Consumer assistant adapters | PDF embedding verification | Verified | Production UI checks passed; new app test modules and live native acceptance not run; not deployed |
| Component installation / installer agent | Verified immutable installs, compatible activation, locking and offline resolution | assistant-install | Protocol/schema versions | Verified | Installer: 13 synthetic tests; no real signature/release check |
| App updates / updater agent | Configured compatible fork releases; no upstream fallback | update/custom and consumer updater modules | Signed compatibility metadata | Verified | Updater: final 176 passed / 4 ignored, including policy and incompatible-package regression; WordCraft fork not found, not deployed |
| Delivery / primary | Assistant-only update changes all three panels with unchanged app binaries | Packaging, docs, focused commits | All previous work | Blocked | No published signed packages/source configuration; native linking/installed smoke blocked by disk headroom; not deployed |

## Verification and limitations

Initial inspection: all four worktrees were clean. Current apps pin shared libraries at bf932b91643c287502d539b88f08abd5796c9447. Existing preview binaries disable self-updates. The development volume has approximately 8 GiB free; checks must be scoped and sequential. Shared craftrules instructions were unavailable locally and the repository API returned 404; repository-owned instructions remain applicable.

Initial app rebuilds are required. Compatible helper releases do not rebuild the hosts; new app tool capabilities or bridge protocol requirements can require app releases. macOS is the first supported installation platform.

## Implementation notes

All desktop entry points opt into the shared helper. Reusable libraries retain legacy behavior unless explicitly enabled, so existing embedding/tests are compatible; shared mode never falls back to legacy services. New shared-mode update/reload actions have matching headless commands.

The installer authenticates the manifest inside the signed helper bundle, checks protocol/settings/history ranges, refuses stale concurrent activation and retains previous versions. Helper Ready.version must match the authenticated manifest before rendering/platform output is accepted. UI work remains isolated from document mutation.

Global models/authentication reuse the existing shared store and secret namespace; no user settings or credentials were migrated or rewritten during implementation. Open settings editors are not automatically reloaded; dirty models/connection forms, drafts and pending input prevent process replacement.

Production signing/notarization, release publication, installed app updates, native VoiceOver/IME/clipboard/performance, and a three-app assistant-only binary update demonstration remain unverified. Development checks do not imply those gates passed.

## Final automated evidence

- `cargo test --offline -p craft-assistant -p craft-assistant-bridge -p craft-assistant-install -p craft-update --features craft-assistant/runtime,craft-update/egui -j 2 --quiet`: **228 passed, 4 ignored**, including 21 assistant unit tests, 4 real helper-process tests, 14 bridge tests, 13 installer tests and 176 updater tests. Loopback and synthetic macOS installer fixtures ran outside sandbox restrictions.
- Scoped assistant runtime/all-target Clippy passed with warnings denied; bridge/installer/updater scoped Clippy passed.
- PDF production UI Clippy passed. Word and Grid production UI checks passed against the reviewed local shared source. New consumer test modules were added but not executed: linking them or desktop binaries was deferred once the volume fell below 1 GiB free.
- Only this task's newly created assistant incremental caches and its temporary bridge target were removed to recover space. Existing application targets and installed bundles were preserved.

No release, installed bundle, user configuration, personal chat or credential was changed. WordCraft's personal fork lookup returned 404; no upstream branch or new remote is selected silently. Optional font input and real signing remain release prerequisites.
