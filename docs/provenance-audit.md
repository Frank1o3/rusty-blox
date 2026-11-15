# Code Provenance Audit — Rusty Blox & roblox-runtime

Date: 2026-11-14. Reference repos live under `references/` and are read-only
inputs. Findings below are measured with diff, not inferred from similarity.

## Summary

| Area | Provenance | Action |
|---|---|---|
| `src/apk_update/*` | Adapted from Cordial `cordial-update` (GPL-3.0-or-later), commit `7abbaf5d5f6cc81ca97e448812985d3f180b2420` — admitted in the module header, confirmed by diff | **Replace** (Phase 2, GPL incompatibility) |
| `src/apk_update/certificates.json` | Comment text adapted from `references/cordial/packaging/trust/roblox-signing-certificates.json` | Rewrite comment; keep digest values (public facts) |
| `src/client_settings.rs` | One comment cites "Cordial's investigation" | Reword comment |
| roblox-runtime `native/*.cpp` | Near-verbatim derivative of Cordial's `native/` tree (see `roblox-runtime/docs/porting-plan.md` §License for measured diffs) | Replaced by the ongoing C++→Rust migration (Phases 3–5) |
| roblox-runtime `crates/linker` | Foundations adapted from Eclipse (MIT), attributed in `crates/linker/ECLIPSE-NOTICE.md` | Keep; MIT→BSD-3-Clause is compatible with attribution |
| roblox-runtime `crates/jnivm` | Independent Rust implementation; behavioral reference is libjnivm (MIT). `abi/direct_buffer.rs` cites libjnivm internals | Keep; add a MIT provenance notice |
| roblox-runtime `third_party/libbadcpu`, `mocktail-webview` | Checked in, referenced by no build file | Delete |
| roblox-runtime `third_party/mcpelauncher-linker`, `libjnivm` | MIT submodules backing the C++ path | Removed only after the Rust linker/JNI VM are the verified default |

Everything else in the client (`src/*.rs` outside `apk_update/`) and the Rust
crates is independently developed; no Cordial-derived code was found there.

## Rusty Blox — borrowed files in detail

`src/apk_update/` (2,197 lines) implements APK acquisition from APKPure with
signature verification. Diff measurements against `cordial-update`:

- `apk_signature.rs` (739 lines): near-verbatim copy of cordial's
  `apk_signature.rs` (953 lines). Differences are rustfmt wrapping and
  `Cordial`→`rusty-blox` string changes. Complete implementation copy.
- `url_policy.rs` (372): adapted from cordial's (488). Same allow-list design,
  manual redirect walker, resolution logic, and error shapes; Cordial-branded
  comments and ADR-025 references throughout.
- `provider/mirror.rs` (844): adapted from cordial's `provider/mirror.rs`
  (1,028). Same APKPure metadata reader and download flow; env var names
  renamed (`CORDIAL_MIRROR_URL` → `RUSTY_BLOX_APK_MIRROR_URL`).
- `provider/mod.rs` (78): adapted from cordial's `provider/mod.rs` (trait,
  `Progress`, `Archives`, `Cancel` types).
- `mod.rs` (164): new orchestration written for Rusty Blox, but its header
  documents the Cordial origin commit.

Licensing: Cordial is GPL-3.0-or-later; Rusty Blox is BSD-3-Clause. The
borrowed code cannot remain under the intended license. The module header
itself says the code is adapted from Cordial, so the provenance is
undisputed. This is the primary replacement target (task §2).

## roblox-runtime

- `native/` C++ tree (audio backends, JNI shims, class hooks, liblog,
  system-path interception) is a near-verbatim Cordial derivative — the
  runtime's own `docs/porting-plan.md` measured the diffs (e.g. `opensles.cpp`
  45 differing lines of 1,655). The migration plan replaces this entire layer
  with the existing Rust crates; the license problem resolves only if the
  Rust ports are independent implementations, not translations of the GPL
  text. Behavioral use of the C++ as an oracle is fine; line-level porting is
  not.
- `crates/linker` (Rust) is a partial implementation gated behind
  `USE_EXPERIMENTAL_RUST_LINKER`; `crates/jnivm` (Rust) likewise behind
  `USE_EXPERIMENTAL_JNIVM`. The default build/link pipeline
  (`crates/linker-sys` build.rs → CMake) still compiles the Cordial-derived
  C++ and the two MIT submodules. Migration status and remaining gaps are
  tracked in `docs/porting-plan.md` §B/§C and `docs/client-migration.md`.
- `crates/audio` (Rust, cpal-based) already covers backend selection, PCM
  queueing, capture, OSS, the AAudio surface and an OpenSL ES port; it is not
  yet the provider of the runtime's audio symbol tables.

## Decisions recorded for the migration

- The pinned Roblox signing-certificate digests are retained: they are public
  facts about Roblox's APK, not Cordial authorship.
- Cordial's GPL text is used strictly as a behavioral reference; no GPL file
  is translated line-by-line into the replacements.
- MIT-sourced ports (Eclipse, libjnivm) are allowed with attribution notices,
  which `crates/linker` has and `crates/jnivm` will gain.
