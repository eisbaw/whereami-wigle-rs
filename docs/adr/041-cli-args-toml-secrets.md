# ADR-041: CLI args for operations, TOML for secrets

**Status**: Accepted
**Date**: 2026-05-03

## Context

The daemon has two kinds of configuration: operational tuning (intervals, thresholds, paths) and secrets (API keys). Mixing them in one file creates deployment friction.

## Decision

All operational parameters are CLI args with sensible defaults. Only API credentials live in a TOML config file (`--config` points to it).

## Rationale

- CLI args are visible in `ps`, overridable in systemd unit files, and self-documenting via `--help`. No guessing what config file format to use.
- Secrets in a separate TOML file can have restricted file permissions (600). They don't change between deployments — only the operational params do.
- No single config file mixing "how often to scan" with "WiGLE API key." Different change cadences, different access patterns.
- Defaults are conservative and work out of the box. Most users never need to touch them.

## Consequences

- Long command lines if overriding many params. Mitigated by sensible defaults and systemd unit file readability.
- Config file must exist for WiGLE to work (or lookups are disabled with a warning). Daemon still starts without it.
