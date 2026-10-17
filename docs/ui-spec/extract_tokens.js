// Extract the WorkBuddy design tokens into an EveryBuddy-owned token file.
//
// Input : WorkBuddy AI 5.8.4 renderer/assets/tokens-*.css (unpacked, read-only)
// Output: docs/ui-spec/tokens.css
//
// Policy: the *values* (colour hex, spacing px, cubic-bezier, shadow maths) are
// facts observed from the shipped product and are re-published under our own
// namespace (--wb-* -> --eb-*). No proprietary source text is copied verbatim.

const fs = require('fs');
const path = require('path');

const SRC = process.argv[2];
const OUT = process.argv[3];

let css = fs.readFileSync(SRC, 'utf8');
css = css.replace(/\/\*[\s\S]*?\*\//g, '');

// The file is flat: every block is `selector{decls}` with no nesting.
const blocks = [];
for (let i = 0; i < css.length; ) {
  const open = css.indexOf('{', i);
  if (open < 0) break;
  const close = css.indexOf('}', open);
  if (close < 0) break;
  blocks.push({ sel: css.slice(i, open).trim(), body: css.slice(open + 1, close) });
  i = close + 1;
}

const light = new Map();
const dark = new Map();
const other = new Map();
// NOTE: the trailing `;` must be optional. Minified CSS omits it on the last
// declaration of every block -- 103 tokens live there, --wb-shadow-dialog among
// them, and requiring the semicolon silently drops them.
const DECL = /(--wb-[a-z0-9-]+)\s*:\s*([^;]+)/g;

for (const { sel, body } of blocks) {
  const isDark = /dark/i.test(sel);
  const isBase = sel.includes(':root');
  const target = isBase ? light : isDark ? dark : other;
  let m;
  DECL.lastIndex = 0;
  while ((m = DECL.exec(body)) !== null) {
    target.set(m[1], m[2].trim());
  }
}

const remap = (s) => s.replace(/--wb-/g, '--eb-');
const emit = (map) => [...map.entries()].sort(([a], [b]) => a.localeCompare(b))
  .map(([k, v]) => `  ${remap(k)}: ${remap(v)};`).join('\n');

const mixed = [...light, ...dark]
  .filter(([, v]) => /color-mix|rgba\(|#/.test(v) && /color-mix/.test(v));

const header = `/* EveryBuddy design tokens
 *
 * Derived from the design-token stylesheet shipped with WorkBuddy AI 5.8.4
 * (com.workbuddy.workbuddy-ai, Electron). Only values were extracted; names are
 * re-namespaced from --wb-* to --eb-*.
 *
 * Buckets
 *   :root                        base layer, ${light.size} declarations
 *   [data-theme="dark"]          dark overrides, ${dark.size} declarations
 *   other (see notes)            ${other.size} declarations, context-scoped
 *
 * NOT YET PORTED: ${other.size} context-scoped declarations (e.g. modal overlay
 * reset) and every value using color-mix() (${mixed.length} in base+dark) --
 * the latter must be pre-computed into literals for the Chromium-109 baseline.
 */

`;

const out = [
  header,
  ':root {',
  emit(light),
  '}',
  '',
  '[data-theme="dark"] {',
  emit(dark),
  '}',
  '',
].join('\n');

fs.mkdirSync(path.dirname(OUT), { recursive: true });
fs.writeFileSync(OUT, out);

console.log('blocks          :', blocks.length);
console.log('base (:root)    :', light.size);
console.log('dark overrides  :', dark.size);
console.log('context-scoped  :', other.size);
console.log('color-mix() uses:', mixed.length);
console.log('wrote           :', OUT, `(${(out.length / 1024).toFixed(1)} KB)`);
console.log('other selectors :', [...new Set(blocks.filter(b => !b.sel.includes(':root') && !/dark/i.test(b.sel)).map(b => b.sel))].join(' | '));
