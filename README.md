# rusty-blox

`rusty-blox` is the desktop client and bootstrapper for `roblox-runtime`.
It owns APK discovery/import, managed files, host window creation and desktop
input forwarding.

Run `rusty-blox` with no arguments (or `cargo run --release`) to open the
launcher. The Home screen can download or update Roblox; **Play Roblox** saves
the current options and starts the client. Settings, sessions, and FastFlags
are available in the launcher, so normal startup does not require CLI flags.

`roblox-runtime` owns Android compatibility, game startup, Android input
translation and graphics backend selection. The client supplies the runtime
with the APK/native-library locations, data and cache paths, configuration and
a renderable host surface. The runtime does not discover APKs or create the
client's top-level window.

The current binary accepts a base APK path, reuses its managed installation,
or looks for Sober's x86-64 `base.apk` and `split_config.x86_64.apk`, then
copies them and extracts `lib/x86_64/*.so` from the APK set into
`$XDG_DATA_HOME/rusty-blox/roblox` (or `~/.local/share/rusty-blox/roblox`).
Unchanged APK source files are reused on later starts.
Run `rusty-blox --update-apk` to download and install the latest x86-64 build
from APKPure. The download is checked against Roblox's pinned APK signing
certificate before it can replace the managed installation. Roblox does not
provide a public generic Android APK download endpoint; use `--import-sober` to
refresh from a Google Play build installed through Sober, or supply an APK path.
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

Pass a base APK as the positional argument, use Sober's installed build, or run
`--update-apk`. Use `--import-sober` to refresh an existing managed build from
Sober. The client also accepts `--fast-flags FILE` and `--client-settings FILE`
and passes those inputs in `RuntimeConfig`. The General tab's host-libc checkbox
opts into the ABI-unsafe diagnostic symbol resolver; it is off by default.

Without `--fast-flags`, the client loads `fast-flags.json` beside its settings
file (`$XDG_CONFIG_HOME/rusty-blox`, or `~/.config/rusty-blox`). The FastFlags
tab edits that JSON directly. General and Game frame-cap controls stay linked;
the selected cap is applied to the loaded FastFlags and XML. FastFlags changes
are saved by the Settings window; the running client does not write its startup
snapshot back on exit. The Game tab exposes common Roblox settings as checkboxes
and sliders backed by `data/files/appData/GlobalBasicSettings_13.xml`, without
launching Roblox.
The client uses `StartScreenSize` for its initial window size, limited to fit
the display. On X11 it requests a floating dialog window; on Wayland, the
compositor controls whether the window floats or tiles.
Focused Roblox text boxes support Ctrl+A, Ctrl+C, Ctrl+X, and Ctrl+V; the system
clipboard uses `wl-clipboard`, `xclip`, or `xsel` when available.

Before Roblox starts, the runtime keeps the newest ten Roblox log files and ten
`memProfStorage*.json` files, then removes empty directories under its data and
cache roots. It leaves `rbx-storage` directories, content blobs, database files,
settings, and session data intact.

Open the launcher and use the Sessions tab to create a named login profile.
Select it and use **Save & Play** to save it as the default for later launches.
The default is “No saved session”. A new profile signs in through Roblox on
its first launch and saves its cookies when the client exits. `--settings` is
still available to open the same launcher explicitly.
`--session NAME` overrides the selected profile for one launch. Only one
Roblox process can run at a time because the engine and JNI state are
process-global.

The General tab in `rusty-blox --settings` selects the JNI backend. C++
libjnivm is the default. If Rust JNI is selected, the checkbox controls whether
unhandled methods and fields can fall back to C++ libjnivm; fallback is enabled
by default. The OpenGL ES swap interval can be set to off (0), on (1), or
adaptive (-1, if the host driver supports it). These settings apply on the next
launch. The General tab also has independent log category checkboxes for base,
runtime, JNI/JNI VM, linker, stubs, input, WebView, and session messages. Base
and runtime messages are enabled by default. The WASD option
lets the most recently pressed key win when opposite directions are held; when
it is released, a still-held opposite key resumes immediately.

## Mini FastFlags list for low end devices
```json
{
  "FFlagDebugGraphicsPreferVulkan": true,
  "DFFlagDebugRenderForceTechnologyVoxel": true,
  "DFFlagDebugPauseVoxelizer": false,
  "DFIntCSGLevelOfDetailSwitchingDistance": 50,
  "DFIntCSGLevelOfDetailSwitchingDistanceL12": 75,
  "DFIntCSGLevelOfDetailSwitchingDistanceL23": 100,
  "DFIntCSGLevelOfDetailSwitchingDistanceL34": 150,
  "DFIntDebugFRMQualityLevelOverride": 1,
  "DFIntTaskSchedulerTargetFps": 120,
  "FFlagDisablePostFx": true,
  "FIntDebugForceMSAASamples": 0,
  "FIntFRMMaxGrassDistance": 0,
  "FIntFRMMinGrassDistance": 0,
  "FIntRenderGrassDetailStrands": 0,
  "FIntRenderGrassHeightScaler": 0,
  "FIntRenderLocalLightFadeInMs": 0,
  "FIntRenderLocalLightUpdatesMax": 1,
  "FIntRenderLocalLightUpdatesMin": 1,
  "FIntRenderShadowIntensity": 0,
  "DFFlagUseVisBugChecks": true,
  "DFFlagRenderBeamOcclusionCulling": true,
  "DFFlagRenderEmitterOcclusionCulling": true,
  "DFFlagRenderFastClusterOcclusionCulling4": true,
  "FFlagLightOcclusionCull5": true,
  "FFlagRenderEnableGlobalInstancingGPUWhitelistAndroid": true,
  "DFIntPerformanceControlTextureQualityBestUtility": -1,
  "FFlagAutomaticDRS4": true,
  "FFlagAutomaticDRSUseGpuTime2": true,
  "FFlagAutomaticDRSEnableLowDPI": true,
  "FFlagRenderDynamicResolutionScale12": true,
  "FFlagNewLightAttenuation3": true
}
```

For development, `USE_EXPERIMENTAL_JNIVM=1` or `true` overrides the saved JNI
backend choice and enables Rust JNI for that launch. `./dev.sh` uses that
override and replaces `rusty-blox.log` with the combined output while also
showing it in the terminal. Enable host-libc in Settings to use the ABI-unsafe
diagnostic resolver for that run.
