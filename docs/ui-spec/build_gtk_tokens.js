// Compile EveryBuddy design tokens into GTK-consumable form.
//
// GTK3's CSS engine supports neither CSS custom properties (`var()`) nor
// color-mix(); it knows @define-color, and its own mix()/shade()/alpha().
// So we resolve the token graph ahead of time:
//
//   tokens.css (--eb-*, var() + color-mix())
//        -> tokens.resolved.json      every token, fully literal
//        -> gtk/tokens-light.css      @define-color set (base theme)
//        -> gtk/tokens-dark.css       @define-color set (dark overrides)
//
// color-mix() is computed with premultiplied-alpha sRGB interpolation, which is
// what CSS specifies for `in srgb` (the only mode the source uses).

const fs = require('fs');
const path = require('path');

const SRC = process.argv[2];
const OUTDIR = process.argv[3];

// ---------------------------------------------------------------- css parsing

const css = fs.readFileSync(SRC, 'utf8');

/** Split a flat `sel{body}` stylesheet into blocks. */
function blocks(src) {
  const out = [];
  for (let i = 0; i < src.length; ) {
    const open = src.indexOf('{', i);
    if (open < 0) break;
    const close = src.indexOf('}', open);
    if (close < 0) break;
    out.push({ sel: src.slice(i, open).trim(), body: src.slice(open + 1, close) });
    i = close + 1;
  }
  return out;
}

function readVars(body) {
  const map = new Map();
  // Trailing `;` optional: the last declaration of a block has none.
  const re = /(--eb-[a-z0-9-]+)\s*:\s*([^;]+)/g;
  let m;
  while ((m = re.exec(body)) !== null) map.set(m[1], m[2].trim());
  return map;
}

const base = new Map();
const dark = new Map();
for (const b of blocks(css)) {
  const target = b.sel.includes(':root') ? base : /dark/i.test(b.sel) ? dark : null;
  if (!target) continue;
  for (const [k, v] of readVars(b.body)) target.set(k, v);
}

// ---------------------------------------------------------------- color maths

const NAMED = { transparent: [0, 0, 0, 0], black: [0, 0, 0, 1], white: [255, 255, 255, 1] };

function parseColor(str) {
  const s = str.trim().toLowerCase();
  if (NAMED[s]) return NAMED[s];
  if (s.startsWith('#')) {
    const h = s.slice(1);
    const n = (i, w) => parseInt(h.slice(i, i + w).padEnd(w, h.slice(i, i + w)), 16);
    if (h.length === 3 || h.length === 4) {
      return [n(0, 1) * 17, n(1, 1) * 17, n(2, 1) * 17, h.length === 4 ? n(3, 1) * 17 / 255 : 1];
    }
    if (h.length === 6 || h.length === 8) {
      return [n(0, 2), n(2, 2), n(4, 2), h.length === 8 ? n(6, 2) / 255 : 1];
    }
    return null;
  }
  const m = s.match(/^rgba?\(([^)]+)\)$/);
  if (m) {
    const p = m[1].split(/[,\s/]+/).filter(Boolean).map(Number);
    if (p.length < 3 || p.some(Number.isNaN)) return null;
    return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
  }
  return null;
}

/** CSS `color-mix(in srgb, c1 p1%, c2 p2%)` — premultiplied alpha. */
function colorMix(c1, p1, c2, p2) {
  let w1 = p1, w2 = p2;
  if (w1 == null && w2 == null) { w1 = w2 = 50; }
  else if (w1 == null) w1 = 100 - w2;
  else if (w2 == null) w2 = 100 - w1;
  const t = w1 + w2;
  w1 /= t; w2 /= t;

  const a = c1[3] * w1 + c2[3] * w2;
  if (a === 0) return [0, 0, 0, 0];
  const ch = (i) => (c1[i] * c1[3] * w1 + c2[i] * c2[3] * w2) / a;
  return [ch(0), ch(1), ch(2), a];
}

const round = (x) => Math.max(0, Math.min(255, Math.round(x)));

function fmt(c) {
  const [r, g, b, a] = [round(c[0]), round(c[1]), round(c[2]), c[3]];
  return a >= 1
    ? `#${[r, g, b].map((v) => v.toString(16).padStart(2, '0')).join('')}`
    : `rgba(${r}, ${g}, ${b}, ${a === 0 ? 0 : Number(a.toFixed(3))})`;
}

// ------------------------------------------------------------ expression walk

/** Index of the `)` matching the `(` that precedes `from`. */
function matchParen(s, from) {
  let d = 0;
  for (let i = from; i < s.length; i++) {
    if (s[i] === '(') d++;
    else if (s[i] === ')') { d--; if (d === 0) return i; }
  }
  return -1;
}

/** Split on commas that are not inside parentheses. */
function splitTop(s) {
  const out = [];
  let d = 0, start = 0;
  for (let i = 0; i < s.length; i++) {
    if (s[i] === '(') d++;
    else if (s[i] === ')') d--;
    else if (s[i] === ',' && d === 0) { out.push(s.slice(start, i)); start = i + 1; }
  }
  out.push(s.slice(start));
  return out;
}

function resolveExpr(expr, vars, cache, stack) {
  let s = expr;

  // innermost-first: repeatedly expand var(), then color-mix()
  for (let guard = 0; guard < 256; guard++) {
    // Process the RIGHTMOST function start first: that is the innermost call, so
    // a var() nested inside a color-mix() is expanded before the mix is judged.
    const mixAt = s.lastIndexOf('color-mix(');
    const varAt = s.lastIndexOf('var(');

    if (varAt > mixAt) {
      const end = matchParen(s, varAt + 3);
      if (end < 0) break;
      const inner = splitTop(s.slice(varAt + 4, end));
      const name = inner[0].trim();
      const fallback = inner.length > 1 ? inner.slice(1).join(',').trim() : '';
      let value = fallback;
      if (vars.has(name)) value = resolveExpr(vars.get(name), vars, cache, stack + [name]);
      else if (!value) value = 'inherit';
      s = s.slice(0, varAt) + value + s.slice(end + 1);
      continue;
    }

    if (mixAt >= 0) {
      // `color-mix(` is 10 characters, so its `(` sits at mixAt + 9.
      const end = matchParen(s, mixAt + 9);
      if (end < 0) break;
      const parts = splitTop(s.slice(mixAt + 10, end));
      const body = parts.slice(1).join(',').trim(); // drop "in srgb"
      const operands = splitTop(body);
      const parsed = operands.map((o) => {
        const t = o.trim();
        const pct = t.match(/([\d.]+)%\s*$/);
        const colorStr = pct ? t.slice(0, pct.index).trim() : t;
        return { c: parseColor(colorStr), p: pct ? parseFloat(pct[1]) : null, raw: t };
      });
      if (parsed.length === 2 && parsed[0].c && parsed[1].c) {
        s = s.slice(0, mixAt) + fmt(colorMix(parsed[0].c, parsed[0].p, parsed[1].c, parsed[1].p)) + s.slice(end + 1);
      } else {
        // Operands are not plain colours -- bail out rather than mangle the text.
        break;
      }
      continue;
    }
    break;
  }
  return s;
}

function resolveAll(vars) {
  const cache = new Map();
  const out = new Map();
  const unresolved = [];
  for (const name of [...vars.keys()].sort()) {
    const v = resolveExpr(vars.get(name), vars, cache, [name]);
    out.set(name, v);
    if (/var\(|color-mix\(/.test(v)) unresolved.push(name);
  }
  return { out, unresolved };
}

const light = resolveAll(base);
const darkResolved = resolveAll(dark);

// -------------------------------------------------------------------- emit

const gtkName = (n) => n.replace(/^--eb-/, '').replace(/-/g, '_');

/// One `@define-color` line for a token, or null when the value is not a colour.
///
/// The colour is re-emitted in a canonical form instead of being passed through
/// verbatim: GTK's @define-color parser rejects 8-digit hex (`#rrggbbaa`),
/// which the source tokens use freely, and a single unparsable line makes GTK
/// discard the *entire* stylesheet.
function defineColor(token, value) {
  const colour = parseColor(value.trim());
  return colour ? `@define-color ${gtkName(token)} ${fmt(colour)};` : null;
}

function gtkDefines(map) {
  return [...map.entries()]
    .sort(([a], [b]) => a.localeCompare(b))
    .filter(([k]) => k.startsWith('--eb-'))
    .map(([k, v]) => defineColor(k, resolveExpr(v, map, new Map(), [k])))
    .filter(Boolean)
    .join('\n');
}

// GTK needs the dark file to carry *only* what changes, wrapped in a class so
// the app can swap providers at runtime.
const darkChanged = [...darkResolved.out.entries()]
  .filter(([k, v]) => light.out.get(k) !== v)
  .sort(([a], [b]) => a.localeCompare(b));

const header = (extra) => `/* EveryBuddy tokens for GTK — GENERATED by build_gtk_tokens.js
 * Source: tokens.css (WorkBuddy AI 5.8.4, --wb-* values re-namespaced)
 * GTK3 CSS has no var()/color-mix(); everything below is a resolved literal.
 * ${extra}
 */
`;

fs.mkdirSync(path.join(OUTDIR, 'gtk'), { recursive: true });

const lightLines = gtkDefines(light.out);
const darkLines = darkChanged.map(([k, v]) => defineColor(k, v)).filter(Boolean);

fs.writeFileSync(
  path.join(OUTDIR, 'gtk', 'tokens-light.css'),
  header(`Light theme: ${light.out.size} tokens, ${lightLines.split('\n').length} colours.`) +
    '\n' + lightLines + '\n'
);

fs.writeFileSync(
  path.join(OUTDIR, 'gtk', 'tokens-dark.css'),
  header(`Dark overrides only: ${darkLines.length} of ${darkResolved.out.size} tokens differ.`) +
    '\n' + darkLines.join('\n') + '\n'
);

const json = {
  source: 'WorkBuddy AI 5.8.4 tokens-*.css',
  light: Object.fromEntries([...light.out.entries()].sort()),
  dark: Object.fromEntries([...darkResolved.out.entries()].sort()),
};
fs.writeFileSync(path.join(OUTDIR, 'gtk', 'tokens.resolved.json'), JSON.stringify(json, null, 1) + '\n');

// ------------------------------------------------------------- metrics emit
//
// Colours are emitted as @define-color. Numbers have no such GTK facility, so
// the same resolved token set is emitted twice more:
//   gtk/tokens-metrics.json  machine-readable spacing/radius/type scale
//   gtk/tokens-metrics.rs    Rust constants, included by crates/eb/src/spec.rs
//
// The values are literal (tokens.css is already resolved). Components can use
// the constants for GtkBox spacing/margins and the JSON for generated CSS.

const METRIC_GROUPS = {
  spacing: '--eb-spacing-',
  radius: '--eb-radius-',
  font_size: '--eb-font-size-',
  font_line_height: '--eb-font-line-height-',
  font_weight: '--eb-font-weight-',
};

function metricNumber(value) {
  const s = value.trim();
  const px = s.match(/^(-?\d+(?:\.\d+)?)px$/);
  if (px) return Number(px[1]);
  if (/^-?\d+(?:\.\d+)?$/.test(s)) return Number(s);
  return null;
}

const metrics = {};
for (const [group, prefix] of Object.entries(METRIC_GROUPS)) {
  metrics[group] = {};
  for (const [name, value] of [...light.out.entries()].sort()) {
    if (!name.startsWith(prefix)) continue;
    const number = metricNumber(value);
    if (number == null || !Number.isInteger(number)) continue;
    metrics[group][name] = number;
  }
}

fs.writeFileSync(
  path.join(OUTDIR, 'gtk', 'tokens-metrics.json'),
  JSON.stringify(
    {
      source: 'WorkBuddy AI 5.8.4 tokens-*.css',
      note: 'Literal metric tokens; values are CSS px or unitless weights.',
      metrics,
    },
    null,
    1
  ) + '\n'
);

const rustName = (name) => name.replace(/^--eb-/, '').replace(/-/g, '_').toUpperCase();
const rustGroup = (group) =>
  `#[allow(dead_code)]\npub mod ${group} {\n` +
  Object.entries(metrics[group])
    .map(([name, value]) => `    pub const ${rustName(name)}: i32 = ${value};`)
    .join('\n') +
  '\n}\n';

const rust = `// GENERATED by docs/ui-spec/build_gtk_tokens.js — do not edit.
// Source: docs/ui-spec/tokens.css (WorkBuddy AI 5.8.4 values, renamed).
// Included by crates/eb/src/spec.rs; spacing/radius/type constants live here.

` + Object.keys(METRIC_GROUPS).map(rustGroup).join('\n');

fs.writeFileSync(path.join(OUTDIR, 'gtk', 'tokens-metrics.rs'), rust);

// Utility classes for values GTK3 cannot express as variables. New components
// add these classes instead of repeating font sizes/radii as literals.
const cssName = (name) => name.replace(/^--eb-/, '').replace(/-/g, '-');
const metricCss = [];
for (const [name, value] of Object.entries(metrics.radius)) {
  metricCss.push(`.eb-${cssName(name)} { border-radius: ${value}px; }`);
}
for (const [name, value] of Object.entries(metrics.font_size)) {
  metricCss.push(`.eb-fs-${cssName(name).replace('font-size-', '')} { font-size: ${value}px; }`);
}
for (const [name, value] of Object.entries(metrics.font_weight)) {
  metricCss.push(`.eb-fw-${cssName(name).replace('font-weight-', '')} { font-weight: ${value}; }`);
}
fs.writeFileSync(
  path.join(OUTDIR, 'gtk', 'tokens-metrics.css'),
  `/* GENERATED by docs/ui-spec/build_gtk_tokens.js — do not edit.
 * Utility classes for numeric tokens; GTK3 CSS has no var().
 */
` + metricCss.join('\n') + '\n'
);

console.log('base tokens      :', base.size);
console.log('dark tokens      :', dark.size);
console.log('dark differing   :', darkChanged.length);
console.log('gtk @define-color:', lightLines.split('\n').filter(Boolean).length, '(light)');
console.log('metric tokens    :', Object.values(metrics).reduce((n, group) => n + Object.keys(group).length, 0));
console.log('unresolved left  :', light.unresolved.length + darkResolved.unresolved.length);
if (light.unresolved.length) console.log('  e.g.', light.unresolved.slice(0, 6).join(', '));
