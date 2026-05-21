---
id: TASK-0167
title: Pin devShell toolchain to the same one nix-package uses
status: To Do
assignee: []
created_date: '2026-05-20 21:33'
labels:
  - tooling
  - nix
dependencies: []
---

## Description

<!-- SECTION:DESCRIPTION:BEGIN -->
Forge F5. flake.nix:25-29 uses fenix.latest.* (moving nightly). nix/package.nix uses pkgsFor system (stable from nixpkgs-unstable). Clippy/rustfmt output in shell can diverge from nix build. Foot-gun for 'just e2e' specifically.
<!-- SECTION:DESCRIPTION:END -->

## Acceptance Criteria
<!-- AC:BEGIN -->
- [ ] #1 Define rustToolchain ONCE (via fenix or rust-overlay, matching MSRV from task-0164)
- [ ] #2 devShells.default.buildInputs uses rustToolchain
- [ ] #3 nix/package.nix passes rustToolchain into craneLib.overrideToolchain
- [ ] #4 Verify: cargo --version inside 'nix develop' and inside 'nix build' produce the same version
- [ ] #5 Nightly stays available for fuzz via devShells.fuzz or rustup-run
<!-- AC:END -->
