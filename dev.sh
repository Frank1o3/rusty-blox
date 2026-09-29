#!/usr/bin/env bash
set -uo pipefail

cd "$(dirname "$(realpath "$0")")"
log_file="$PWD/rusty-blox.log"

set +e
USE_EXPERIMENTAL_JNIVM=1 cargo run -- --host-libc "$@" 2>&1 | tee "$log_file"
status=${PIPESTATUS[0]}
set -e
exit "$status"
