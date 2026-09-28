# rusty-blox

`rusty-blox` is the desktop client and bootstrapper for `roblox-runtime`.
It will own APK discovery and import (including Sober-managed APKs), copy
required files into its managed data area, load client settings, create the host
window and forward host events to the runtime.

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
both the host EGL window and Roblox's surface callbacks. The current runtime
still has no render context or event pump, and keyboard/mouse forwarding is
pending, so reaching GameActivity startup does not establish that a playable
game appears.
