#!/usr/bin/env bash
#
# Runs cargo in a Rust container: nothing is installed on the host. The
# registry and the git checkouts are cached in $REPORT_CARGO_HOME (default:
# ~/.cache/phrasea-report-cargo), the build in ./target. The image is the
# rust-dev stage of the Dockerfile (rustfmt, clippy).
#
# MEM_M (MB) and JOBS, when set, bound the container's memory and CPUs.
#
# Usage: bin/cargo.sh <cargo arguments>
#   e.g. bin/cargo.sh test

set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CARGO_CACHE="${REPORT_CARGO_HOME:-${HOME}/.cache/phrasea-report-cargo}"
IMAGE="${REPORT_RUST_IMAGE:-phrasea-report-rust-dev}"

mkdir -p "${CARGO_CACHE}"

# The rust-dev stage of the Dockerfile copies nothing: built without context.
if [ -z "${REPORT_RUST_IMAGE:-}" ]; then
    docker build -q --target rust-dev -t "${IMAGE}" - < "${DIR}/Dockerfile" >/dev/null
fi

DOCKER_FLAGS=()
if [ -t 0 ] && [ -t 1 ]; then
    DOCKER_FLAGS+=(-it)
fi
if [ -n "${MEM_M:-}" ]; then
    DOCKER_FLAGS+=(--memory="${MEM_M}m" --memory-swap="${MEM_M}m")
fi
if [ -n "${JOBS:-}" ]; then
    DOCKER_FLAGS+=(--cpus="${JOBS}" -e CARGO_BUILD_JOBS="${JOBS}")
fi

exec docker run --rm "${DOCKER_FLAGS[@]}" \
    --user "$(id -u):$(id -g)" \
    --network host \
    -v "${DIR}:/work" -w /work \
    -v "${CARGO_CACHE}:/cargo" \
    -e CARGO_HOME=/cargo \
    -e CARGO_TERM_COLOR="${CARGO_TERM_COLOR:-auto}" \
    -e PATH=/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin \
    "${IMAGE}" cargo "$@"
