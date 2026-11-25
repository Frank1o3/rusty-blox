# Contributing to Rusty Blox

Rusty Blox is the Linux desktop bootstrapper and client. The Roblox Android
compatibility runtime is maintained in the roblox-runtime submodule.

## Before opening an issue or pull request

- Read the Code of Conduct.
- Check existing issues and pull requests for duplicate reports.
- Keep reports reproducible and distinguish observed behavior from guesses.
- Do not include passwords, session cookies, authentication headers, private
  account data, or unredacted logs containing secrets.

## Development setup

The project uses Rust 2024 and depends on GTK4, WebKitGTK 6, Wayland/X11
integration, and native components built by the runtime. Install the
development packages for those libraries, plus a current stable Rust toolchain,
Clang/Clang++, CMake, and the audio/graphics development headers required by
your Linux distribution.

Clone the repository with its runtime submodule:

~~~sh
git clone --recurse-submodules https://github.com/Frank1o3/rusty-blox.git
cd rusty-blox
~~~

If the repository was cloned without submodules:

~~~sh
git submodule update --init --recursive
~~~

## Checks

Run these from the bootstrapper root:

~~~sh
cargo fmt --all -- --check
cargo check
cargo test
cargo build --release
~~~

The runtime is a separate Cargo workspace under roblox-runtime/. When
changing runtime code, run its workspace checks from that directory as well:

~~~sh
cd roblox-runtime
cargo fmt --all -- --check
cargo check --workspace
cargo test --workspace
~~~

Build and test on a supported x86-64 Linux host. The runtime currently rejects
non-x86-64 hosts; AArch64-specific ABI code is not end-to-end platform support.

## Changes to the runtime submodule

Runtime changes belong in
[Frank1o3/roblox-runtime](https://github.com/Frank1o3/roblox-runtime), not as
uncommitted edits embedded in a bootstrapper pull request.

1. Make and test the change in the runtime repository.
2. Push the runtime commit.
3. Update the bootstrapper's roblox-runtime submodule pointer to that commit.
4. Include the submodule pointer change in the bootstrapper pull request and
   explain the runtime change.

Do not update the submodule pointer to a commit that is not available from the
runtime repository.

## Pull requests

- Explain the problem and the behavior changed.
- Keep changes focused; separate refactors from behavior changes.
- Add or update tests for changed behavior.
- Update user or developer documentation when interfaces, build requirements,
  or workflows change.
- Record the exact commands run and any checks you could not run.
- Preserve upstream copyright and license notices. Review provenance before
  copying or adapting third-party code.
- Avoid changing dependency versions or the lockfile without a concrete reason.

## Security and privacy

The APK updater handles untrusted metadata and archive bytes. Preserve HTTPS
validation, redirect validation, download size limits, APK signature
verification, and fail-closed behavior. Never weaken certificate checks to make
a download succeed. Do not log or commit session cookies, tokens, or private
account information.

For vulnerabilities, follow SECURITY.md rather than opening a public issue with
exploit details.
