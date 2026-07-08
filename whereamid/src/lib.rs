//! whereamid library: exposes modules for integration and property tests.

pub mod apple;
pub mod boot_clock;
pub mod db;
pub mod debounce;
pub mod geo;
pub mod http;
pub mod quality;
pub mod scanner;
pub mod trilaterate;

// Shared test-only helpers, declared under both crate roots (lib + bin) so
// `crate::test_support::…` resolves in either test binary (task-0251).
#[cfg(test)]
mod test_support;
