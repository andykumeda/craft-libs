# Assistant message-box outline

Updated 2026-11-05. Status: **Verified source; release packaging in progress**.

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
| Candidate package | In progress | Optimized helper0.1.7 with exact existing local signer, reviewed font pin37827adc41fd165f9a9234ac40f94dbad353aa69 and protocol1; no public release/notarization. |
| Both-Mac installation | Not started | Coordinator owns activation, installed-path rendering and per-host evidence. |

Regression: `composer_outline_is_visible_when_idle_and_focused_in_both_themes` in
`crates/ui/src/assistant/tests.rs`. Visual capture: `export_widget_screenshots` (explicitly ignored
by normal tests). Private PNGs are under the host temporary directory with names
`craft-assistant-pane{,-light,-focus,-light-focus}.png`; no screenshot is committed or published.

Logs: `/private/tmp/craft-assistant017-{outline-red,outline-green,ui-tests,visuals,helper-tests,clippy,wasm,release}.log`.
Durable reviewed build input: `~/.local/share/craft-build-inputs/helper-fonts-37827`.
Physical VoiceOver, native IME/clipboard and live provider inference were not repeated for this
visual-only fix. All existing data and update/activation safeguards remain in place.
