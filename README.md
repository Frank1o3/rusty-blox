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

The current binary is a placeholder; APK import, window creation, surface
handoff and event forwarding are not implemented yet.
