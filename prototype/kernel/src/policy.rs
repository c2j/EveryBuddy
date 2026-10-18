//! The search-policy seam: a waterfall that decides whether a scan may run.
//!
//! dsh gates filesystem mutations through a single-slot waterfall
//! (`fs/write-intent`) where a listener either decides or delegates with
//! `next()`. The core grows the same shape for searches: before the walker
//! starts, the `SearchRequested` event flows through registered gates, and a
//! gate that does not call `next()` vetoes the scan with a reason.
//!
//! The default gate refuses roots outside the filesystem mount. Without it, a
//! request for an unmounted path would return a confusing empty success; with
//! it, the refusal names the reason, and the policy lives in a disposable
//! plugin a deployment can replace rather than in the request handler.
use rutis::{BoxFuture, CordisError, Ctx, Disposer, Effect, Event, EventKey, Next, Plugin, WaterfallListener};
use serde::Serialize;

use crate::fs::FsService;

/// One scan about to start.
pub struct SearchRequested {
    /// Root the scan would walk, as the host spelled it.
    pub root: String,
}

/// What the chain decided.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchDecision {
    Allow,
    /// Refuse; the reason is what the host reports.
    Deny(String),
}

impl Event for SearchRequested {
    const NAME: &'static str = "evobuddy/search-requested";
    type Value = SearchDecision;
}

/// The default gate: a scan must stay inside the mounted filesystem.
///
/// The mount is read from the registry at decision time, not captured at
/// registration, so a provider reload changes what the gate consults without
/// re-registering it.
struct MountGate;

impl WaterfallListener<SearchRequested> for MountGate {
    fn call<'a>(
        &'a self,
        ctx: &'a Ctx,
        e: &'a SearchRequested,
        next: Next<'a, SearchRequested>,
    ) -> BoxFuture<'a, Result<SearchDecision, CordisError>> {
        Box::pin(async move {
            let Some(fs) = ctx.get::<FsService>() else {
                // No filesystem provider: let the scan proceed and fail the way
                // a missing capability always fails here.
                return next.call().await;
            };
            match fs.contains_dir(&e.root) {
                Ok(true) => next.call().await,
                Ok(false) => Ok(SearchDecision::Deny(format!(
                    "\"{}\" is outside the mounted root \"{}\"",
                    e.root,
                    fs.root().display()
                ))),
                Err(failure) => Ok(SearchDecision::Deny(failure.message)),
            }
        })
    }
}

/// Registers [`MountGate`]. Disposing the plugin lifts the gate.
pub struct SearchGate;

impl Plugin for SearchGate {
    fn name(&self) -> &str {
        "search-gate"
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let disposer: Disposer = ctx
                .events()
                .on_waterfall(ctx, &EventKey::of(), MountGate)?;
            ctx.effect(move || {
                drop(disposer);
                Effect::Done
            })?;
            Ok(Effect::Done)
        })
    }
}

/// The terminal continuation: with no veto, a scan is allowed.
pub fn allow_terminal<'a>(
    _: &'a Ctx,
    _: &'a SearchRequested,
) -> BoxFuture<'a, Result<SearchDecision, CordisError>> {
    Box::pin(async move { Ok(SearchDecision::Allow) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use crate::fs::FsProvider;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("evobuddy-policy-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Await errors arrive as `Arc<CordisError>`; the aggregate variant carries
    /// them without unwrapping the shared error.
    fn plugin_error(error: Arc<CordisError>) -> CordisError {
        CordisError::Aggregate { errors: vec![error] }
    }

    fn decide(
        ctx: &Ctx,
        runtime: &tokio::runtime::Runtime,
        root: &str,
    ) -> Result<SearchDecision, String> {
        let request = SearchRequested {
            root: root.to_string(),
        };
        runtime.block_on(async {
            ctx.events()
                .waterfall(ctx, &EventKey::of(), &request, allow_terminal)
                .await
                .map_err(|e| e.to_string())
        })
    }

    #[test]
    fn the_default_gate_denies_roots_outside_the_mount_and_allows_inside() {
        let root = scratch("gate");
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let ctx = Ctx::root_with(runtime.handle().clone());
        runtime
            .block_on(async {
                (&ctx.plugin(FsProvider { root: root.clone() }))
                    .await
                    .map_err(plugin_error)?;
                (&ctx.plugin(SearchGate)).await.map_err(plugin_error)?;
                Ok::<(), CordisError>(())
            })
            .unwrap();

        let inside = decide(&ctx, &runtime, root.display().to_string().as_str()).unwrap();
        assert_eq!(inside, SearchDecision::Allow);

        let outside = root.parent().unwrap();
        let denied = decide(&ctx, &runtime, outside.display().to_string().as_str()).unwrap();
        assert!(
            matches!(&denied, SearchDecision::Deny(reason) if reason.contains("outside the mounted root")),
            "expected a denial naming the mount, got {denied:?}"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    /// A gate that vetoes without calling `next`, so everything after it is
    /// unreachable.
    struct Veto;

    impl WaterfallListener<SearchRequested> for Veto {
        fn call<'a>(
            &'a self,
            _: &'a Ctx,
            _: &'a SearchRequested,
            _next: Next<'a, SearchRequested>,
        ) -> BoxFuture<'a, Result<SearchDecision, CordisError>> {
            Box::pin(async move { Ok(SearchDecision::Deny("vetoed".to_string())) })
        }
    }

    /// A gate that records it ran, then delegates.
    struct Trailing {
        ran: std::sync::Arc<std::sync::Mutex<bool>>,
    }

    impl WaterfallListener<SearchRequested> for Trailing {
        fn call<'a>(
            &'a self,
            _: &'a Ctx,
            _: &'a SearchRequested,
            next: Next<'a, SearchRequested>,
        ) -> BoxFuture<'a, Result<SearchDecision, CordisError>> {
            Box::pin(async move {
                if let Ok(mut ran) = self.ran.lock() {
                    *ran = true;
                }
                next.call().await
            })
        }
    }

    #[test]
    fn a_veto_stops_the_chain_before_later_listeners_and_the_terminal() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let ctx = Ctx::root_with(runtime.handle().clone());
        let ran = std::sync::Arc::new(std::sync::Mutex::new(false));
        runtime
            .block_on(async {
                ctx.events()
                    .on_waterfall(&ctx, &EventKey::of(), Veto)
                    .map(drop)?;
                ctx.events()
                    .on_waterfall(&ctx, &EventKey::of(), Trailing { ran: ran.clone() })
                    .map(drop)?;
                Ok::<(), CordisError>(())
            })
            .unwrap();

        let decision = decide(&ctx, &runtime, "/anything").unwrap();

        assert_eq!(decision, SearchDecision::Deny("vetoed".to_string()));
        assert!(!*ran.lock().unwrap(), "the vetoed listener must never run");
    }

    #[test]
    fn lifting_a_gate_returns_the_chain_to_the_terminal() {
        let root = scratch("lift");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let ctx = Ctx::root_with(runtime.handle().clone());
        let disposer = runtime
            .block_on(async {
                let disposer = ctx.events().on_waterfall(&ctx, &EventKey::of(), Veto)?;
                Ok::<_, CordisError>(disposer)
            })
            .unwrap();

        let before = decide(&ctx, &runtime, "/anything").unwrap();
        assert_eq!(before, SearchDecision::Deny("vetoed".to_string()));

        // rutis disposers are explicit: dropping schedules nothing, so lifting
        // the gate is an awaited call.
        runtime.block_on(disposer.dispose()).unwrap();
        let after = decide(&ctx, &runtime, "/anything").unwrap();
        assert_eq!(after, SearchDecision::Allow);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
