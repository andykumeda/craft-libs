# Shared assistant installation

`craft-assistant-install` installs one copy of the assistant for all desktop app hosts. The
initial delivery supports macOS. It never modifies an app bundle, starts a service, downloads
anything, chooses a provider, or edits conversations. Apps approve/download a selected release
on a worker and pass its local archive, version and SHA-256 to `Store::install`.

See [the installation contract](../../docs/assistant-install.md) for package layout, trust,
configuration, activation, recovery and verification limits.
