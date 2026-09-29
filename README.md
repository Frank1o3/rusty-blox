# rusty-blox

`rusty-blox` is the desktop client and bootstrapper for `roblox-runtime`.
It owns APK discovery/import, managed files, host window creation and desktop
input forwarding.

`roblox-runtime` owns Android compatibility, game startup, Android input
translation and graphics backend selection. The client supplies the runtime
with the APK/native-library locations, data and cache paths, configuration and
a renderable host surface. The runtime does not discover APKs or create the
client's top-level window.

The current binary accepts a base APK path or looks for Sober's x86-64
`base.apk` and `split_config.x86_64.apk`, then copies them and extracts
`lib/x86_64/*.so` from the APK set into
`$XDG_DATA_HOME/rusty-blox/roblox` (or `~/.local/share/rusty-blox/roblox`).
Unchanged APK source files are reused on later starts.
The client builds a `RuntimeConfig` from those managed APK, library, data and
cache paths, prepares Android filesystem and asset views, then creates a winit
window on X11 or Wayland. For Wayland it owns a `wl_egl_window` for as long as
the runtime surface is installed. It asks the runtime to map the engine, run
its deferred constructors, initialise JNI and GameActivity, perform the
app-bridge startup calls, and deliver the first surface. Resize events update
both the host EGL window and Roblox's surface callbacks. Mouse, a mapped
keyboard subset, and committed text for focused Roblox text boxes are
forwarded to the runtime. The runtime event pump and render context remain
unfinished, so reaching GameActivity startup does not establish that a
playable game appears.

Pass a base APK as the positional argument, or use Sober's installed build. The
client also accepts `--fast-flags FILE` and `--client-settings FILE` and passes
those inputs in `RuntimeConfig`. `--host-libc` opts into the runtime's
ABI-unsafe diagnostic symbol resolver; it is off by default.

Use `rusty-blox --settings` and open the Sessions tab to create a named login
profile, select it, and click **Set as default session** to use it on later
launches. The default is “No saved session”. A new profile signs in through
Roblox on its first launch and saves its cookies when the client exits.
`--session NAME` overrides the selected profile for one launch. Only one
Roblox process can run at a time because the engine and JNI state are
process-global.

The General tab in `rusty-blox --settings` selects the JNI backend. C++
libjnivm is the default. If Rust JNI is selected, the checkbox controls whether
unhandled methods and fields can fall back to C++ libjnivm; fallback is enabled
by default. The OpenGL ES swap interval can be set to off (0), on (1), or
adaptive (-1, if the host driver supports it). These settings apply on the next
launch.

For development, `USE_EXPERIMENTAL_JNIVM=1` or `true` overrides the saved JNI
backend choice and enables Rust JNI for that launch. `./dev.sh` uses that
override, runs with `--host-libc`, and replaces `rusty-blox.log` with the
combined output while also showing it in the terminal.
