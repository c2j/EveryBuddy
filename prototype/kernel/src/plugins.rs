//! rutis plugins: the core's own extension points.
//!
//! Two plugins, one provider and one consumer. The consumer declares
//! `Deps = (Arc<SearchEngine>,)`, so rutis gates its start on the engine being
//! present and **restarts it when a new provider replaces the old one**. That
//! restart is the hot-plug behaviour: the host swaps the pattern set and the
//! dependent plugin comes back on its own, with no reload call and no restart
//! of the core.
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rutis::{BoxFuture, CordisError, Ctx, Effect, Plugin, TypedPlugin};
use serde::Serialize;

use crate::host;
use crate::search::{SearchEngine, SearchLimits};

/// A setup failure, surfaced as `CordisError::PluginFailed` so rutis can roll
/// the load back instead of leaving a half-registered plugin.
#[derive(Debug)]
struct SetupFailed(String);

impl std::fmt::Display for SetupFailed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SetupFailed {}

fn setup_failed(message: impl Into<String>) -> CordisError {
    CordisError::PluginFailed(Box::new(SetupFailed(message.into())))
}

/// Provides a compiled [`SearchEngine`] as a typed service.
pub struct SearchProvider {
    /// Patterns to compile into the matcher.
    pub patterns: Vec<String>,
    /// Bounds carried by the engine.
    pub limits: SearchLimits,
}

impl Plugin for SearchProvider {
    fn name(&self) -> &str {
        "search-provider"
    }

    fn apply<'a>(&'a self, ctx: &'a Ctx) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let engine = SearchEngine::new(self.patterns.clone(), self.limits.clone())
                .map_err(setup_failed)?;
            let pattern_count = engine.patterns().len();
            ctx.provide(engine)?;
            host::log(
                "info",
                &format!("search provider compiled {pattern_count} patterns"),
            );
            Ok(Effect::Done)
        })
    }
}

/// One sweep the consumer performed, recorded so the host can see reloads.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepReport {
    /// The pattern set the engine was compiled with when this sweep ran.
    pub patterns: Vec<String>,
    pub files_scanned: usize,
    pub total_matches: usize,
    pub bytes_scanned: u64,
    pub elapsed_ms: f64,
    /// First few matches, so a reload shows real content and not just counts.
    pub preview: Vec<crate::search::Hit>,
}

/// Depends on the engine and sweeps on every (re)start.
///
/// This is the plugin the host watches: its report list grows by one each time
/// the provider is replaced.
pub struct SweepReporter {
    /// The tree to sweep.
    pub root: PathBuf,
    /// Sink the host reads back through `search.reports`.
    pub reports: Arc<Mutex<Vec<SweepReport>>>,
    /// How many hits to keep per sweep.
    pub preview_len: usize,
}

impl TypedPlugin for SweepReporter {
    type Deps = (Arc<SearchEngine>,);

    fn name(&self) -> &str {
        "sweep-reporter"
    }

    fn apply<'a>(
        &'a self,
        _ctx: &'a Ctx,
        (engine,): Self::Deps,
    ) -> BoxFuture<'a, Result<Effect, CordisError>> {
        Box::pin(async move {
            let outcome = engine.find(&self.root).map_err(setup_failed)?;
            let patterns = engine.patterns().to_vec();
            let report = SweepReport {
                patterns,
                files_scanned: outcome.files_scanned,
                total_matches: outcome.total_matches,
                bytes_scanned: outcome.bytes_scanned,
                elapsed_ms: outcome.elapsed_ms,
                preview: outcome.hits.into_iter().take(self.preview_len).collect(),
            };
            host::log(
                "info",
                &format!(
                    "sweep finished: {} matches across {} files",
                    report.total_matches, report.files_scanned
                ),
            );
            if let Ok(mut reports) = self.reports.lock() {
                reports.push(report);
            }
            Ok(Effect::Done)
        })
    }
}
