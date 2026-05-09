# Justfile for whereami project
# All recipes run inside nix-shell if shell.nix is present.

nix_run := if path_exists("shell.nix") == "true" { "nix-shell --run" } else { "bash -c" }

build:
    {{nix_run}} "cargo build"

test:
    {{nix_run}} "cargo test"

run *ARGS:
    {{nix_run}} "cargo run --bin whereamid -- {{ARGS}}"

lint:
    {{nix_run}} "cargo clippy -- -D warnings"

fmt:
    {{nix_run}} "cargo fmt --check"

clean:
    {{nix_run}} "cargo clean"

e2e:
    {{nix_run}} "cargo test && cargo clippy -- -D warnings"

# --- Fuzz targets (nightly Rust via flake devShell) ---
# Enter shell: nix develop
# Then run: just fuzz-all

fuzz-iw:
    cd whereamid && cargo fuzz run fuzz_iw_parser

fuzz-nmcli:
    cd whereamid && cargo fuzz run fuzz_nmcli_parser

fuzz-apple:
    cd whereamid && cargo fuzz run fuzz_apple_decode

fuzz-trilat:
    cd whereamid && cargo fuzz run fuzz_trilaterate

# Run all fuzzers for 60s each
fuzz-all:
    cd whereamid && cargo fuzz run fuzz_iw_parser -- -max_total_time=60
    cd whereamid && cargo fuzz run fuzz_nmcli_parser -- -max_total_time=60
    cd whereamid && cargo fuzz run fuzz_apple_decode -- -max_total_time=60
    cd whereamid && cargo fuzz run fuzz_trilaterate -- -max_total_time=60
