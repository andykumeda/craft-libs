# Assistant message-box outline

Updated 2026-11-05. Status: **Verified source and signed candidate; coordinated installation pending**.

The owner requested a clearer border around the message box at the bottom of the PDFCraft
assistant. The shared composer frame previously had only a background fill; its multiline
editor explicitly has no separate frame. Identical light fills and nearly identical dark
fills made the box boundary unclear.

The shared UI now adds the existing theme's opaque neutral foreground stroke around the
rounded composer, and uses the existing selection stroke while the editor has keyboard focus.
This changes only the message box. It does not change the sidebar separator, provider/model
routing, storage, credentials, settings/history schemas or private bridge protocol1.

The fix ships in shared helper **0.1.7**. Compatible existing hosts, including shared-host
revision83e61084, do not need rebuilding. It applies to the composer in every host using this
shared helper. Older installed0.1.6 remains active until the coordinator explicitly installs
and verifies the candidate on each Mac; source and package checks alone are not deployment.

| Work | State | Evidence / acceptance |
|---|---|---|
| Visible idle/focus outline | Verified | Actual painted rounded frame is opaque and at least1px; contrast against fill and surrounding panel is at least3:1 in both themes; focus outline has a distinct color. |
| Red proof | Verified | New regression failed against the old fill-only frame: width0/alpha0 in light idle. Same regression passes with the outline. |
| UI checks | Verified | craft-ui lib481passed/2ignored; ignored synthetic visual capture explicitly run and passed; light/dark idle/focus PNGs inspected. |
| Helper checks | Verified | craft-assistant all-target tests30passed; scoped all-target Clippy with warnings denied passes for UI/helper; craft-ui WASM check passes; formatting/diff checks pass. Existing unused egui patch warnings remain. |
| Candidate package | Verified | Optimized helper0.1.7 release passed; exact existing signer, immutable temporary-store installer and all four app protocol1 handshakes pass. Reviewed font pin37827adc41fd165f9a9234ac40f94dbad353aa69. No public release/notarization. |
| Both-Mac installation | Not started | Coordinator owns activation, installed-path rendering and per-host evidence. |

Regression: `composer_outline_is_visible_when_idle_and_focused_in_both_themes` in
`crates/ui/src/assistant/tests.rs`. Visual capture: `export_widget_screenshots` (explicitly ignored
by normal tests). Private PNGs are under the host temporary directory with names
`craft-assistant-pane{,-light,-focus,-light-focus}.png`; no screenshot is committed or published.

Logs: `/private/tmp/craft-assistant017-{outline-red,outline-green,ui-tests,visuals,helper-tests,clippy,wasm,release}.log`.
Durable reviewed build input: `~/.local/share/craft-build-inputs/helper-fonts-37827`.
Physical VoiceOver, native IME/clipboard and live provider inference were not repeated for this
visual-only fix. All existing data and update/activation safeguards remain in place.

## Signed candidate

Source commit: `f2295dd9808016cab89e25c091027069f49dcddc`.
Candidate directory: `/private/tmp/craft-assistant-017-20261105/package/`.
Archive `craft-assistant-macos-arm64.tar.gz` SHA-256:
`33ff84d86131119e597127d0d9f81aad1af6a5f556fa83e8657c37cb7458b23b`.
Signed executable SHA-256:
`f73b7e74f2db1f8e69e72c2dcf7621f164a3c3c0f5bc1e4ba730c2ee45a77d39`.
Certificate leaf: `D7B1B5524FFEB37AB01D3960017BDF6743E3EDFD`;
bundle identifier: `local.craft.shared-assistant`.
The authenticated manifest retains protocol/settings/history range1. The package was signed,
strictly verified, installed only into a disposable trusted store, and independently initialized
for PDFCraft, WordCraft, GridCraft and DeckCraft with version0.1.7/protocol1 replies. No production
active pointer or application was changed by qualification. Release build completed in6m14s.

The coordinator's existing installer route is prepared in
`/private/tmp/craft-assistant-017-20261105/install-helper.py`; it checks the receipt/archive,
uses the existing certificate-pinned immutable installer, preserves the previous helper,
verifies four installed handshakes and checks public configuration/app hashes are unchanged.
Copy only the installer script and package archive/receipt/installer executable to the other
host; do not copy user profiles, credentials or conversations. Installed-path rendering and
per-host activation evidence remain separate acceptance checks.

GitHub temporarily rejected the first two existing-fork pushes with an Internal Server Error;
read-only verification confirmed the previous remote source was unchanged. A later push
succeeded: sourcef2295dd and qualificationda1a96e were published to the existing
`fork/codex/independent-assistant` branch, and remote SHA da1a96eab8e583cab0d11e0facddb0f13791ae45
was independently verified. No public release was created.
