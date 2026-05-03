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
