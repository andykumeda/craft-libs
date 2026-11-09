# Assistant-compatible shared libraries: 2026-11-09

Status: **Verified source candidate; not deployed.** This merge combines assistant source
`83e61084dee3117d34f95600d6ec68bad8b0b9b5` with official shared source
`1b34eba41e62138fc2438d2734523b302840b45d`. It preserves the assistant's existing Codex
isolation verification and integrates upstream updater behavior. Application-specific source,
packaging, signed artifacts, and installation require their own verification.

The regenerated lockfile adds the assistant dependency graph while retaining all upstream
package names. Most version changes add parallel versions; `zbus_names` 4.3.5 became 4.3.4,
and `zvariant`/`zvariant_derive` 5.15.1 became 5.15.0. These are the exact resolved versions
used by the checks below; downstream consumers should review this Linux dependency drift.

## Verification

- `cargo fmt --all -- --check` passed.
- `cargo clippy --offline --locked --workspace --all-targets -- -D warnings` passed with
  pinned proofing-language input `c239f934ace90b7939afaceb23b472ceeafd6b8a` and font input
  `b076417b5880bd4154bd22f00695b1d28c6bea83`. The font consumer emitted warnings for its
  synthetic fonts without character maps.
- Workspace tests against the same resolved lock passed in two complementary runs: a serial
  `cargo test --offline --workspace -- --test-threads=1 --skip
  tests::replacement_checks_mode_after_streaming_callback` passed 2,884 tests across 170
  suites with 34 ignored and one filtered. A process-local sandbox denied reads only of the
  host's installed LibreOffice French dictionary. The filtered atomic mode-bit test passed
  separately outside that sandbox (one test), because the sandbox strips its special mode bits.
  No installed dictionary was changed. Ignored tests include live Codex, keychain, network,
  released-app, external-font, benchmark, and documentation fixtures.
- Earlier workspace attempts exposed two timing-dependent assistant cancellation tests; each
  passed on independent retry and in the final serial run. Another run without isolation failed
  three proofing tests that assume French is unavailable; all three failures reproduced on the
  unchanged official 1b34 source and passed with only that dictionary hidden from the process.
  Sandbox-only special-mode failure also passed in the separate unsandboxed test described above.
- Upstream vendor files contain existing trailing whitespace reported by `git diff --cached
  --check`; they were retained unchanged. No native application installation or live model turn
  was performed for this merge.
