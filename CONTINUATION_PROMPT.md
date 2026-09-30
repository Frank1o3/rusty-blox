# Continuation prompt

Continue the active Rusty-Blox text editing and overlay work. Start by reading
this prompt, checking `git status` and the diffs in both the `rusty-blox` and
`roblox-runtime` repositories, then inspect the current `rusty-blox.log` and
the relevant host-window/input code. Preserve all existing user changes.

## Current goal

Text entry into Roblox UI fields now works in the user's run. Add a visible
text editor overlay for focused fields (chat, search, login, numeric input,
etc.), including the current text and caret, positioned using Roblox's
`RawTextBoxInfo`. The active desktop session is Wayland. The overlay must not
intercept mouse or keyboard input. The user also observed 280
`[jnivm:fallback]` log entries and wants those investigated alongside the
overlay work.

## Work already in progress

The `rusty-blox` working tree currently has changes in:

- `src/text_overlay.rs` (new low-level Wayland subsurface and shared-memory
  text/caret painter; not yet compiled or exercised)
- `src/host_window.rs` (Wayland overlay owner and update/hide integration)
- `src/app.rs` (per-frame focused textbox overlay sync)
- `src/main.rs` (module registration)

Review these diffs before extending them. In particular, check all
`wl_proxy_marshal_flags` argument lists against the Wayland protocol signatures,
proxy destruction rules for protocol version 1, buffer release and detach
lifetimes, Cairo pixel format/stride, logical versus physical coordinates,
subsurface stacking, and that the empty input region leaves clicks directed at
the game. Overlay creation is optional: if it cannot be created, the client
should still launch and report why. Current integration only provides a
Wayland overlay; X11 has no overlay implementation.

The overlay currently gets text and caret from `ClientApp`, and geometry from
`roblox_runtime::jni::game_activity::focused_textbox_info()`. Confirm that
focus transitions, edits, cursor movement, resize, and focus loss update or
hide it correctly. Check the numeric/password input type mapping against the
runtime's captured data or an authoritative in-repo reference rather than
guessing. The `RawTextBoxInfo` documentation is in
`roblox-runtime/crates/jni/src/game_activity/lifecycle_text.rs`.

## Fallback log investigation

The reported run had exactly 280 `[jnivm:fallback]` entries: 140 pairs for
`com/google/androidgamesdk/gametextinput/State` fields (`text`, selection
start/end, composing-region start/end). They appear to be repeated field reads
for objects created in the companion C++ VM: Rust has field declarations but
no values for those foreign receivers, and the C++ fallback also cannot resolve
those receiver handles. Check the latest log before assuming these counts or
messages are unchanged.

There is already shared GameTextInput state in `roblox-runtime/native/game_activity.cpp`
and Rust accessors in `roblox-runtime/crates/jni/src/game_activity/`. Determine
whether the current tree already bridges those five field reads from that
snapshot. If so, explain why the logs still occur or identify other fallback
messages; if not, implement the smallest correct bridge, preserving the lean
Rust JNI VM design. Do not implement a full `NewGlobalRef`/JNI VM merely to
silence logs. The user's intended Rust VM behavior is to remain lean and use
logs to reveal new JNI surface area.

## Validation and workflow

Inspect the changes and compile the affected Rust crates/client to catch
integration errors. Do not add tests unless they are needed to cover a concrete
regression. If a live run is available, `./dev.sh` replaces `rusty-blox.log`,
so retain/inspect the old log first and check the new overlay and fallback
behavior after launch. Report what was compiled or run, and distinguish
confirmed behavior from code that could not be exercised in this environment.

Older JNI investigation context: prior work focused on advancing startup with
`USE_EXPERIMENTAL_JNIVM=1`, using the C++ `libjnivm`/runtime as reference for
observed methods and fields. Keep fixes grounded in observed calls or existing
reference implementations. Do not attribute unrelated native stub failures to
JNI without evidence.
