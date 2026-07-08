//! Shared test-only helpers reachable from BOTH build targets.
//!
//! `whereamid` compiles as both a library (`lib.rs`) and a binary (`main.rs`),
//! and several modules (`db`, `trilaterate`, `quality`, …) are declared under
//! both roots, so their `#[cfg(test)]` code runs in TWO separate test binaries.
//! Helpers that those tests share must therefore live in a module declared under
//! both roots. This file is declared `#[cfg(test)] mod test_support;` in BOTH
//! `lib.rs` and `main.rs`; `crate::test_support::…` then resolves in either
//! target (task-0251).

#![cfg(test)]

/// Install, once per process, a permissive global default subscriber so that
/// thread-local (`with_default` / `with_subscriber`) capture tests reliably
/// see their own events under `cargo test`'s parallel harness (task-0248).
///
/// ROOT CAUSE: tracing caches a process-global per-callsite `Interest`. That
/// cache is (re)built by folding every registered dispatcher's
/// `register_callsite` and defaulting to `Interest::never()` when the fold is
/// empty (tracing-core 0.1.36 `rebuild_callsite_interest`). A callsite that is
/// only ever exercised by ONE scoped-subscriber test — e.g. the non-finite
/// skip `warn!` in `handle_locate` — can be first-registered in a window where
/// that test's dispatcher is not yet visible to the fold, caching it as
/// `never`. Thereafter `warn!` short-circuits BEFORE consulting the active
/// dispatcher, the event is silently dropped, and the scoped capture sees `[]`
/// (measured ~12% under full-bin parallelism; two thread-local-only fixes
/// failed because they never touched the dispatcher registry the fold reads).
///
/// FIX (algebraic, not probabilistic): a global default built via
/// `Dispatch::new` is registered into that same registry, and
/// `set_global_default` leaks it so its weak ref never dies — it is therefore
/// a permanent member of the dispatcher set the interest fold reads. Its
/// `register_callsite` returns `Interest::sometimes()`, and `Interest::and`
/// collapses any mismatched pair to `sometimes()`, so no fold can ever cache a
/// callsite as `never`. A `sometimes` callsite always evaluates `enabled()`
/// against the CURRENT thread-local dispatcher, so the scoped capture
/// deterministically sees its own events regardless of cross-thread timing.
///
/// This subscriber is a no-op sink: `event()` does nothing and `enabled()` is
/// false, so it never records and can never FABRICATE an event in any scoped
/// capture. What it deliberately DOES change is that genuinely-emitted events
/// at previously-`never`-cached callsites are no longer silently dropped — so
/// sibling capture tests become deterministic instead of occasionally lossy.
/// It cannot hide a real absence (it records nothing), so absence assertions
/// stay sound; the only behavioural change is "lost real event" -> "seen real
/// event".
///
/// task-0251 hoisted this out of `server::dispatch_tests` into this shared,
/// lib+bin-reachable module and wired it into EVERY tracing-capture test
/// (server, db, trilaterate, config) so the guarantee holds regardless of run
/// shape, test order, or which test binary (lib vs bin) is running — not only
/// the full-`just e2e` bin run where a server capture test happened to install
/// it first.
pub(crate) fn install_permissive_global_test_subscriber() {
    use std::sync::Once;
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        struct Permissive;
        impl tracing::Subscriber for Permissive {
            fn register_callsite(
                &self,
                _metadata: &'static tracing::Metadata<'static>,
            ) -> tracing::subscriber::Interest {
                // The whole point: force every callsite to `sometimes` so the
                // global interest cache can never short-circuit `warn!`.
                tracing::subscriber::Interest::sometimes()
            }
            fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
                false
            }
            fn new_span(&self, _attrs: &tracing::span::Attributes<'_>) -> tracing::span::Id {
                tracing::span::Id::from_u64(1)
            }
            fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}
            fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {
            }
            fn event(&self, _event: &tracing::Event<'_>) {}
            fn enter(&self, _span: &tracing::span::Id) {}
            fn exit(&self, _span: &tracing::span::Id) {}
        }
        // No other code sets a global default in these test binaries (main()'s
        // fmt init does not run under the harness), so this succeeds and the
        // `sometimes()` guarantee holds. HONEST FAILURE MODE: if some future
        // code (a dep, or a test that calls `fmt().init()`) installs a global
        // default first, this returns `Err`, the `Once` is still consumed, and
        // the de-flake silently no-ops — the flake would quietly return with no
        // signal. If that ever happens, this must become a fail-loud install
        // (assert the resulting global yields `sometimes()`), not a silent
        // fallback. We keep the swallow only because that precondition is
        // currently guaranteed for these binaries.
        let _ = tracing::subscriber::set_global_default(Permissive);
    });
}

/// One captured tracing event as `(level, target, message)`.
///
/// This is the SUPERSET shape: 3-field call sites (server, trilaterate) filter
/// on `target`; 2-field call sites (db, config, scanner, history) ignore the
/// `target` column and map down to `(level, message)`. Consolidating on the
/// superset lets ONE sink serve all six former per-module `CapturedEvents` /
/// `Captured` structs (task-0255).
pub(crate) type CapturedEvent = (tracing::Level, String, String);

/// Test-only tracing subscriber that records every event's
/// `(level, target, message)` into a shared, cheaply-clonable buffer.
///
/// Clones share the same buffer (`Arc<Mutex<…>>`), so the idiom is: build one,
/// clone a handle, install the original as the scoped subscriber
/// (`with_default` / `with_subscriber`), run the code under test, then snapshot
/// via the retained handle's [`CaptureSink::events`]. `enabled()` returns
/// `true` so it records everything; it can never fabricate an event, so absence
/// assertions stay sound. Most sync call sites should prefer [`capture`], which
/// wires up the permissive global and the scoped install in one call.
#[derive(Clone, Default)]
pub(crate) struct CaptureSink(std::sync::Arc<std::sync::Mutex<Vec<CapturedEvent>>>);

impl CaptureSink {
    /// Snapshot the events captured so far, in emission order.
    pub(crate) fn events(&self) -> Vec<CapturedEvent> {
        self.0.lock().expect("capture lock poisoned").clone()
    }

    /// Inject a synthetic event into the buffer in the current capture order.
    ///
    /// Used by db.rs's SQLite commit-hook marker to interleave a `"test
    /// commit"` marker with the genuinely-emitted `warn!` so a test can assert
    /// the commit precedes the warning. Not a tracing event — a direct buffer
    /// write from a non-tracing callback.
    pub(crate) fn record_event(&self, level: tracing::Level, target: &str, message: &str) {
        self.0.lock().expect("capture lock poisoned").push((
            level,
            target.to_owned(),
            message.to_owned(),
        ));
    }
}

/// Extracts the `message` field from a tracing event. Copied verbatim from the
/// (now-deleted) per-module visitors; lives here once (task-0255).
struct MessageVisitor(String);

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }

    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }
}

impl tracing::Subscriber for CaptureSink {
    fn enabled(&self, _metadata: &tracing::Metadata<'_>) -> bool {
        true
    }

    fn new_span(&self, _attributes: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _span: &tracing::span::Id, _values: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _span: &tracing::span::Id, _follows: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let mut visitor = MessageVisitor(String::new());
        event.record(&mut visitor);
        self.0.lock().expect("capture lock poisoned").push((
            *event.metadata().level(),
            event.metadata().target().to_owned(),
            visitor.0,
        ));
    }

    fn enter(&self, _span: &tracing::span::Id) {}
    fn exit(&self, _span: &tracing::span::Id) {}
}

/// Install the permissive global (task-0251), run `f` under a scoped
/// [`CaptureSink`], and return `f`'s result plus the captured events.
///
/// This is the one-call ergonomic path for the common SYNC capture pattern.
/// Async sites that attach the sink to a future (`WithSubscriber`) need the raw
/// [`CaptureSink`] instead, since the scoped guard must span the whole future.
pub(crate) fn capture<T>(f: impl FnOnce() -> T) -> (T, Vec<CapturedEvent>) {
    install_permissive_global_test_subscriber();
    let sink = CaptureSink::default();
    let handle = sink.clone();
    let result = tracing::subscriber::with_default(sink, f);
    (result, handle.events())
}
