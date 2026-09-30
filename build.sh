#!/bin/sh
# Build a static x86_64 Linux binary in Docker (works from macOS, e.g. with
# OrbStack). Output: target/linux/x86_64-unknown-linux-musl/release/p1505-duplex
set -e
cd "$(dirname "$0")"
docker run --rm --platform linux/amd64 \
    -v "$PWD":/src -w /src \
    -v p1505-duplex-cargo:/usr/local/cargo/registry \
    rust:alpine sh -c '
        apk add --quiet musl-dev &&
        cargo build --release --locked \
            --target x86_64-unknown-linux-musl --target-dir target/linux'
ls -l target/linux/x86_64-unknown-linux-musl/release/p1505-duplex
