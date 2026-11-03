# Independently installed assistant bundles

Status: installation foundation implemented, 2026-11-02. App integration, signed release
publication and real installed-app update acceptance are separate gates. This document describes
`craft-assistant-install`, not a claim that the suite already uses it.

## Package and compatibility contract

A release is a gzip-compressed tar archive containing only `CraftAssistant.app/`. Its executable
is `Contents/MacOS/craft-assistant`. The signed resource
`Contents/Resources/assistant-manifest.json` contains:

```json
{
  "version": "1.0.0",
  "protocol": { "min": 1, "max": 1 },
  "settings": { "min": 1, "max": 1 },
  "history": { "min": 1, "max": 1 }
}
```

All format ranges are positive, inclusive and ordered. Each host passes its actual protocol,
settings and history versions in `HostCompatibility`. Installation and activation refuse any
incompatible range. The signed manifest version must equal the selected release and installed
directory version. The helper handshake must independently confirm the negotiated wire contract.
The existing unversioned conversation JSON is history format `1`; preserving format `1` means
preserving its fields and meanings without an irreversible migration. These declarations do not
perform a migration or authorize one.

Release builders must sign the complete `.app`, including this manifest and every resource,
with the configured publisher identity, notarize it, then create the tarball and publish its
SHA-256. No symbolic/hard links, duplicate paths, device files or other special entries are
accepted. Package frameworks that require symlinks are outside this first delivery.

## Configuration and trust

Callers supply `Config` from their documented configuration source. It requires an absolute
shared installation `root`, `publisher_team_id` and `signing_identifier`. There is no default
publisher. The resolved root must be outside `.app` bundles. A source configured for one
publisher must never substitute another feed or publisher when verification fails.

`Limits::default` is the sole source of default limits, overridable through `Config.limits`:

| Limit | Default |
| --- | --- |
| Compressed package | 128 MiB |
| Total extracted file bytes | 256 MiB |
| Archive entries | 512 |
| Manifest/activation metadata | 64 KiB |
| Each macOS verification command | 120 seconds |

Configuration validation enforces supported safety ceilings: 1 GiB compressed, 2 GiB
unpacked, 100,000 entries, 1 MiB metadata and 300 seconds per verification command. Values must
be positive. These ceilings prevent a corrupt configuration disabling resource bounds.

The compressed checksum verifies download integrity against the selected release, **not**
publisher authenticity. The installer runs `/usr/bin/codesign --verify --strict --deep` with an
Apple trust anchor, pinned team and pinned signing identifier; then `/usr/sbin/spctl --assess
--type execute` must approve the bundle. Verification commands use `craft-update`'s bounded
subprocess runner. There is no production bypass for unsigned or ad-hoc-signed packages.
Publisher checks use safe argument passing, never shell interpolation. On other platforms,
installation fails explicitly as unsupported.

The host reads component configuration from `<AI shared root>/component.json`; the AI root
continues to honor `CRAFT_AI_CONFIG_DIR`. `schema_version` must be `1`. Required `installation`
fields are those above. Required `source` fields are `repo` (`owner/repository`) and `asset_name`
(the exact platform `.tar.gz` filename). Optional `bridge` overrides bridge limits as a complete
object; defaults are 16 MiB messages, 4,194,304 texture pixels, 500,000 vertices and 256 input
events. `startup_timeout_secs` defaults to 15 and accepts 1–300. Unknown component fields and
unsupported schema versions fail without rewriting the file or selecting another feed.
Release checks do not install anything. They require a stable newer version, the exact asset,
a positive declared size within the configured compressed limit, a repository-confined download
URL, and a valid SHA-256 from GitHub's digest or bounded `SHA256SUMS.txt`. Only explicit approval
calls the download/install path. The existing `craft-update` HTTP client enforces the declared
size and checksum while downloading into temporary storage.

## Building a signed macOS release

Build `cargo build --release -p craft-assistant --features runtime`. Package the resulting
`target/release/craft-assistant` as `CraftAssistant.app/Contents/MacOS/craft-assistant`, retaining
its executable permission. Create `Contents/Info.plist` with `CFBundlePackageType=APPL`,
`CFBundleExecutable=craft-assistant`, the configured identifier in `CFBundleIdentifier`, and the
release version in `CFBundleShortVersionString` and `CFBundleVersion`. Put the manifest above in
`Contents/Resources/assistant-manifest.json`. The release version must match the helper's Cargo
package version and its `Ready.version` handshake; set the package version before building.

Sign the complete bundle using your own configured Developer ID identity:

```sh
codesign --force --options runtime --timestamp --sign "$ASSISTANT_SIGNING_IDENTITY" CraftAssistant.app
codesign --verify --strict --deep CraftAssistant.app
ditto -c -k --keepParent CraftAssistant.app CraftAssistant-notarization.zip
xcrun notarytool submit CraftAssistant-notarization.zip --keychain-profile "$ASSISTANT_NOTARY_PROFILE" --wait
xcrun stapler staple CraftAssistant.app
spctl --assess --type execute CraftAssistant.app
tar -czf "$ASSISTANT_ASSET_NAME" CraftAssistant.app
shasum -a 256 "$ASSISTANT_ASSET_NAME"
```

Use a stable exact `ASSISTANT_ASSET_NAME` per target architecture matching `source.asset_name`.
Configure notarization credentials in the platform credential store separately; do not put them
in the repository or configuration. Publish the archive and its checksum as a release in the
configured repository only after the separate release approval. The commands above are a recipe,
not evidence that any signing, notarization or publication has been performed.

## Activation and recovery

The store contains `versions/<version>/CraftAssistant.app`, `active.json` and `install.lock`.
Install/activation take a cross-process OS file lock; all app instances use the same configured
root. Packages extract into fresh private staging directories. Extraction is bounded, every
file is flushed, the completed version is renamed into place, and `craft-atomic` commits the
active pointer. Existing versions are never overwritten or automatically removed. `active.json`
records the current version and immediately previous active version.
An update selected before another app installs a newer release cannot downgrade that activation;
the caller must refresh its update check. Rollbacks use explicit `Store::activate`.

`Store::resolve` returns an absolute executable path, version and manifest. It performs an
offline signature check without downloading, invoking Gatekeeper, or selecting a different
version. Missing, damaged or incompatible active installations produce actionable errors. App
hosts retain the returned executable for an existing session and only resolve the current
pointer when starting the next eligible session. Installing a version does not stop a process
that is already running it.

`Store::activate` explicitly reactivates a retained version after checking signature and host
compatibility. This does not roll back documents, settings or chats. Assistant writers must keep
their declared settings/history formats compatible with retained versions. A checksum, signature,
extraction or compatibility failure leaves the previous activation unchanged. An activation-write
failure can leave a complete inactive version, which can be activated explicitly. A killed
installer can leave a private `.assistant-stage-*` directory; the store does not sweep them while
another process might be using one. Remove orphan staging directories only after all installers
have stopped. No version garbage collector is included, so retained versions consume disk space.

## Verification

Focused synthetic tests cover offline resolution, explicit rollback, immutable versions, wrong
checksums, interrupted/corrupt transfers, invalid publisher results, mismatched release metadata,
incompatible format ranges, concurrent installation, extraction limits, duplicate entries, links,
non-bundle entries, corrupt activation data, configured safety ceilings, nested symlinked paths
and stale/concurrent update downgrade prevention. Tests inject the private verifier
function; they do not bypass the production verifier or establish that a real package is signed
or notarized. Real Developer ID signing, Gatekeeper assessment, killed-process recovery, power-loss
durability, and installed-app activation require separate acceptance evidence.

Executed on 2026-11-02: `cargo test --offline -p craft-assistant-install -j 2` passed 13 tests;
`cargo test --offline -p craft-assistant --lib -j 2` passed six configuration/release-selection
tests. Scoped all-target Clippy for both crates with `-D warnings` passed, and the changed Rust
files passed formatting checks. These runs did not download a real release or install a signed
assistant package.

## Revision history

| Date | Change |
| --- | --- |
| 2026-11-02 | Add shared immutable bundle installation and explicit activation contract |
