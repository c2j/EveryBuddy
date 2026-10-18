/**
 * EvoBuddy contract plugin: the dsh plugin shape, one service, Rust behind it.
 *
 * This file is deliberately unremarkable. `inject` names a service, `apply`
 * reads it off the context, and nothing here knows or cares that `evoSearch` is
 * answered by a Rust wasm core two frames away. That is the contract under
 * test: if a dsh plugin compiles against this shape and runs, the same shape
 * carries every real dsh plugin.
 *
 * The probe runs one search over the configured root and reports what came back
 * on the `evo/contract-report` event, so the host can assert on real data
 * without the plugin growing test hooks.
 */

/**
 * Services this plugin requires. Resolved by Cordis before `apply` runs; an
 * unmet name leaves the plugin pending rather than handing `apply` an undefined.
 * @type {string[]}
 */
export const inject = ['evoSearch']

/** Search roots come from the deployment, like any dsh config value. */
export const Config = undefined

/**
 * Probe the search service once at load.
 * @param {import('@deepseek-ai/cordis').Context} ctx context with `evoSearch` resolved
 * @param {{ root?: string, patterns?: string[], maxResults?: number }} config
 */
export function apply(ctx, config = {}) {
  const startedAt = Date.now()
  const outcome = ctx.evoSearch.find({
    ...(config.root !== undefined ? { root: config.root } : {}),
    ...(config.patterns !== undefined ? { patterns: config.patterns } : {}),
    ...(config.maxResults !== undefined ? { maxResults: config.maxResults } : {}),
  })

  ctx.emit('evo/contract-report', {
    patterns: outcome.patterns,
    totalMatches: outcome.totalMatches,
    filesScanned: outcome.filesScanned,
    truncated: outcome.truncated,
    firstHit: outcome.hits[0] ?? null,
    hostRoundTripMs: Date.now() - startedAt,
    coreElapsedMs: outcome.elapsedMs,
  })
}
