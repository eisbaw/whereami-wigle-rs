# Justfile for whereami project.
# Recipes assume you are inside the dev shell: nix-shell or nix develop.
# For one recipe: nix-shell --run 'just <recipe>' or nix develop -c just <recipe>.

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

# Pre-commit gate: shell.nix evaluation, tests, and strict clippy
e2e:
    nix-instantiate shell.nix >/dev/null
    cargo test
    cargo clippy --all-targets -- -D warnings

# Full pre-commit gate including format check and short fuzz smoke
qa: e2e fmt fuzz

# --- Nix flake checks (reproducible, run in the Nix sandbox; task-0209) ---
# These are minutes-long (they build the workspace and boot a NixOS VM), so
# they are deliberately kept OUT of `e2e` — e2e stays the fast pre-commit gate.

# Run all flake checks: package build, clippy (-D warnings), tests, fmt, VM test
nix-check:
    nix flake check -L

# Build just the NixOS VM module test (x86_64-linux, needs KVM); iterate here
vm-test:
    nix build .#checks.x86_64-linux.vm -L

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

# --- Kani bounded-model-checker (task-0082) ---
# Setup is OUT OF BAND because Kani brings its own forked Rust toolchain;
# don't try to run this inside `nix develop`. One-time:
#     cargo install --locked kani-verifier
#     cargo kani setup
# Then `just kani` runs all #[kani::proof] harnesses in whereamid (the only
# crate with proofs; whereami-client has none).
# Individual proofs can be selected with: cd whereamid && cargo kani --harness <name>
kani:
    cd whereamid && cargo kani -p whereamid
