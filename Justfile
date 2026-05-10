# Justfile for whereami project.
# Recipes assume you are inside the dev shell:  nix develop
# (or run a single recipe with:  nix develop -c just <recipe>)

# Build the workspace
build:
    cargo build

# Run the unit + integration test suite
test:
    cargo test

# Run the daemon
run *ARGS:
    cargo run --bin whereamid -- {{ARGS}}

# Strict clippy: any warning is a hard error
lint:
    cargo clippy --all-targets -- -D warnings

# Verify formatting (CI-friendly; does not modify files)
fmt:
    cargo fmt --check

# Auto-format the workspace
fmt-fix:
    cargo fmt

# Remove build artefacts
clean:
    cargo clean

# Pre-commit gate: tests + strict clippy
e2e:
    cargo test
    cargo clippy --all-targets -- -D warnings

# Full pre-commit gate including format check and short fuzz smoke
qa: e2e fmt fuzz

# --- Fuzz targets (nightly Rust via flake devShell) ---
# Each fuzz-<name> recipe runs indefinitely; Ctrl-C to stop.
# `fuzz` is a short smoke (15s/target); `fuzz-all` is 60s/target.

# Fuzz the iw scanner output parser
fuzz-iw:
    cd whereamid && cargo fuzz run fuzz_iw_parser

# Fuzz the nmcli scanner output parser
fuzz-nmcli:
    cd whereamid && cargo fuzz run fuzz_nmcli_parser

# Fuzz the Apple WPS protobuf decoder
fuzz-apple:
    cd whereamid && cargo fuzz run fuzz_apple_decode

# Fuzz the Apple WPS protobuf encoder (length-field invariant)
fuzz-apple-encode:
    cd whereamid && cargo fuzz run fuzz_apple_encode

# Fuzz the trilateration math
fuzz-trilat:
    cd whereamid && cargo fuzz run fuzz_trilaterate

# Quick smoke: 15s on every fuzz target (≈75s total)
fuzz:
    cd whereamid && cargo fuzz run fuzz_iw_parser      -- -max_total_time=15
    cd whereamid && cargo fuzz run fuzz_nmcli_parser   -- -max_total_time=15
    cd whereamid && cargo fuzz run fuzz_apple_decode   -- -max_total_time=15
    cd whereamid && cargo fuzz run fuzz_apple_encode   -- -max_total_time=15
    cd whereamid && cargo fuzz run fuzz_trilaterate    -- -max_total_time=15

# Long smoke: 60s on every fuzz target (≈5 minutes total)
fuzz-all:
    cd whereamid && cargo fuzz run fuzz_iw_parser      -- -max_total_time=60
    cd whereamid && cargo fuzz run fuzz_nmcli_parser   -- -max_total_time=60
    cd whereamid && cargo fuzz run fuzz_apple_decode   -- -max_total_time=60
    cd whereamid && cargo fuzz run fuzz_apple_encode   -- -max_total_time=60
    cd whereamid && cargo fuzz run fuzz_trilaterate    -- -max_total_time=60
