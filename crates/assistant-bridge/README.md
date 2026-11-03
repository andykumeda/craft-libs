# Craft assistant bridge

An embedded assistant panel whose Rust renderer and runtime live in an independently installed
helper process. The application retains its document engine, tool authorization and Undo.
The helper is launched explicitly with `--stdio`, using inherited private pipes; no network
listener or background service is created.

`wire::PROTOCOL` identifies the message schema. Messages are four-byte big-endian length-prefixed
JSON. `Limits` is supplied by the shared assistant configuration and bounds message bytes, input
events, vertices, indices and cumulative texture pixels. Hard maximums additionally protect the
host from an invalid configuration. Invalid input, EOF, broken pipes and queue saturation return
errors; requests are never automatically retried.

The wire uses explicit input events, clipped triangle meshes, premultiplied RGBA textures,
clipboard/URL commands and IME state. It never sends egui's Rust structs or memory layouts.
Accessibility uses AccessKit's public 0.25 node/action schema with its own explicit schema
identifier; incompatible schemas must be refused. The application maps every node ID and relation
into its own namespace, translates the remote root and grafts it into its existing tree.

## Host

1. Verify the installed helper and its protocol compatibility before `Client::spawn`.
2. Send `HostMessage::Initialize` and validate the returned `Ready` version/capabilities.
3. Set the panel's expected signed release version with `set_expected_version` before polling.
   Retain one `EmbeddedPanel` and `Client` per app session. Call `poll(context, client)` while
   the pane is hidden and `show(ui, client)` when visible. Consume `poll_events()` after polling;
   it retains initialization, runtime and tool events while applying every texture delta in order.
4. Route tool requests through the existing app-local authorization and document guards. Keep
   native app keyboard shortcuts out of a focused panel using `owns_keyboard()` before dispatch.
5. Show `error()` with a recovery control. Do not automatically restart or replay an uncertain edit.

Only one frame request is outstanding. Input events are queued in order while it completes;
queue exhaustion is explicit. `input_pending()` remains true until buffered input and its
frame response have drained; update activation must additionally obtain a fresh ordered helper
restart-state reply. A disabled panel flushes existing input without creating new input frames.
Pointer capture continues beyond the panel bounds. Text, paste,
key names/physical keys, modifiers, scrolling phase/source, preedit, UTF-8/character deletions,
IME reset completion, clipboard and accessible actions cross the same boundary. Tab and Shift+Tab
leave the helper at its keyboard boundaries and resume the host's normal focus traversal. The helper's
textures have a separate host namespace, and are freed after the frame that uses them.

## Helper

Enable the helper's default-font feature or install its attributed font stack.
Construct `PanelRenderer::new(limits)`, poll assistant services with `context()`, and draw existing
shared widgets inside `frame(input, |ui| ...)`. The renderer returns `PanelFrame`; send it in
`HelperMessage::Frame(Box::new(frame))`. Rendering callbacks and unmanaged GPU texture IDs are
rejected because they cannot cross a process boundary. Initialize translations and provider
configuration from the separately validated initialization payload.

## Verification

`cargo test -p craft-assistant-bridge -j 2` covers bounded framing, malformed geometry/textures,
actual text editing and Japanese IME commit, Retina rendering output, scroll/delete units, and
host accessibility grafts. `cargo clippy -p craft-assistant-bridge --all-targets -- -D warnings`
is the focused lint gate. These headless checks are not live macOS VoiceOver, native IME, GPU
performance, clipboard or application smoke acceptance; those belong to the consumer rollout.
