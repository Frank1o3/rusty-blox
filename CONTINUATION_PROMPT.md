# Continuation prompt

Continue the experimental Rust JNI VM work in this workspace. Read this file,
the current git diff, `roblox-runtime/crates/jnivm/OBSERVED_SURFACE.md`, the
relevant C++ reference implementations under `roblox-runtime/native/`, and the latest
`rusty-blox.log` before changing code.

The goal is to make `USE_EXPERIMENTAL_JNIVM=1` startup progress by implementing
the actual Java/JNI behaviors still missing from the latest runtime logs. Use
the C++ `libjnivm`/runtime code as the source of truth where behavior exists;
where it does not, document that rather than guessing. Pay particular
attention to missing methods and fields, JNI setters, constructor factories,
and Android input handling. Prior observed failures included
`NativeFlagsInitResult` constructor/addBoolean and an invalid fallback array
reference, missing `Insets` and `Configuration` fields, missing
`MotionEvent`/`KeyEvent` methods, and missing `InputConnection` methods. Recheck
the new log because these may have changed.

Run the client with `./dev.sh [client arguments]`. The script sets
`USE_EXPERIMENTAL_JNIVM=1`, streams output to the terminal, and replaces
`rusty-blox.log` with the current run's combined stdout/stderr. Inspect the log
after each relevant change. Keep fixes scoped to behavior supported by the C++
reference or observed JNI calls, and report unresolved gaps separately. Do not
claim that the experimental VM is working until a run demonstrates it.

The previous run ended at `[stub] ZSTD_trace_decompress_begin`; determine from
the newest log and linker/runtime context whether that native stub issue is
independent of the JNI gaps. Do not attribute it to JNI without evidence.
