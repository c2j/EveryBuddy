// Capture the live layout of WorkBuddy's main workspace over the Chrome
// DevTools Protocol, to use as ground truth when rebuilding the UI in GTK.
//
// Read-only: only DOM/CSS/Page *read* methods are called. Nothing is evaluated
// in the page, and nothing is written into it.
//
//   node capture_dom.js <out-dir>        (CDP_PORT env, default 9333)
//   CDP_SHOT=0 node capture_dom.js <out-dir>     skip the screenshot
//
// Outputs land in <out-dir>, which should be a temp location rather than the
// repo: the dump contains the user's own workspace content.

const fs = require('fs');
const path = require('path');

const PORT = process.env.CDP_PORT || '9333';
const OUT = process.argv[2];
if (!OUT) {
  console.error('usage: node capture_dom.js <out-dir>');
  process.exit(1);
}

const log = (...parts) => console.error('[capture]', ...parts);

// The computed properties actually needed to rebuild a layout in GTK. Anything
// absent here is dropped, which keeps the dump small enough to diff by eye.
const STYLES = [
  'display', 'position', 'box-sizing', 'overflow', 'overflow-x', 'overflow-y', 'z-index', 'opacity',
  'flex-direction', 'flex-wrap', 'flex-grow', 'flex-shrink', 'flex-basis',
  'justify-content', 'align-items', 'align-self', 'gap', 'row-gap', 'column-gap',
  'grid-template-columns', 'grid-template-rows',
  'width', 'height', 'min-width', 'min-height', 'max-width', 'max-height',
  'padding-top', 'padding-right', 'padding-bottom', 'padding-left',
  'margin-top', 'margin-right', 'margin-bottom', 'margin-left',
  'background-color', 'background-image',
  'color', 'border-top-width', 'border-right-width', 'border-bottom-width',
  'border-left-width', 'border-top-color', 'border-radius', 'box-shadow',
  'font-family', 'font-size', 'font-weight', 'line-height', 'letter-spacing',
  'text-align', 'text-overflow', 'white-space', 'transition',
];

class CDP {
  constructor(socket) {
    this.socket = socket;
    this.nextId = 0;
    this.pending = new Map();
    socket.addEventListener('message', (event) => {
      const message = JSON.parse(event.data);
      const entry = this.pending.get(message.id);
      if (!entry) return;
      this.pending.delete(message.id);
      if (message.error) entry.reject(new Error(JSON.stringify(message.error)));
      else entry.resolve(message.result);
    });
  }

  send(method, params = {}, timeoutMs = 30000) {
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      // Every request is bounded. A CDP call that never answers must surface as
      // an error, not as a hung process.
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`${method} timed out after ${timeoutMs}ms`));
      }, timeoutMs);

      this.pending.set(id, {
        resolve: (value) => { clearTimeout(timer); resolve(value); },
        reject: (error) => { clearTimeout(timer); reject(error); },
      });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
}

async function connect(webSocketDebuggerUrl, timeoutMs = 15000) {
  const socket = new WebSocket(webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('websocket open timed out')), timeoutMs);
    const fail = (why) => { clearTimeout(timer); reject(new Error(why)); };
    socket.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
    socket.addEventListener('error', () => fail('websocket error'), { once: true });
    // Chromium refuses CDP sockets whose Origin it dislikes and drops them with
    // a close frame rather than an error, which would otherwise hang forever.
    socket.addEventListener('close', (event) => {
      fail(`websocket closed: code=${event.code} reason=${event.reason || '(none)'}`);
    }, { once: true });
  });
  return new CDP(socket);
}

/**
 * DOMSnapshot packs everything into flat index arrays. Unpack them.
 *
 * Note the two distinct index spaces: `layout.*` arrays are keyed by layout
 * order, while `nodes.parentIndex` is keyed by *node* order. Mixing them up
 * silently yields garbage (and, if the parent chain never terminates, an
 * endless walk), so each entry keeps its node index for parent lookups.
 */
function decode(snapshot) {
  const strings = snapshot.strings;
  const nodes = snapshot.documents[0].nodes;
  const layout = snapshot.documents[0].layout;
  const str = (i) => (typeof i === 'string' ? i : strings[i]);

  const attribute = (nodeIndex, name) => {
    const flat = nodes.attributes[nodeIndex];
    if (!flat) return null;
    for (let i = 0; i + 1 < flat.length; i += 2) {
      if (str(flat[i]) === name) return str(flat[i + 1]);
    }
    return null;
  };

  // `layout.styles[k]` is NOT name/value pairs. It is one value per entry of the
  // requested `computedStyles` list, positionally aligned with it — the names
  // are implied by the request. Pairing it as name/value produces output that
  // looks like real data but is meaningless.
  const stylesOf = (k) => {
    const values = layout.styles[k];
    const out = {};
    if (!values || (values.length === 1 && values[0] === '')) return out;
    for (let i = 0; i < STYLES.length && i < values.length; i++) {
      out[STYLES[i]] = str(values[i]);
    }
    return out;
  };

  return layout.nodeIndex.map((nodeIndex, k) => {
    const box = layout.bounds[k];
    const bounds = Array.isArray(box[0]) ? box[0] : box;
    return {
      nodeIndex,
      parentNodeIndex: nodes.parentIndex[nodeIndex],
      tag: String(str(nodes.nodeName[nodeIndex])).toLowerCase(),
      id: attribute(nodeIndex, 'id') || '',
      className: attribute(nodeIndex, 'class') || '',
      bounds: { x: bounds[0], y: bounds[1], w: bounds[2], h: bounds[3] },
      styles: stylesOf(k),
    };
  });
}

async function main() {
  const response = await fetch(`http://127.0.0.1:${PORT}/json/list`);
  const targets = await response.json();

  log(`${targets.length} CDP target(s)`);
  for (const target of targets) log(`  [${target.type}] ${target.url.slice(0, 90)}`);

  // The main workspace window, if it is the one that got rendered.
  const page =
    targets.find((t) => t.type === 'page' && /renderer\/index\.html/.test(t.url)) ||
    targets.find((t) => t.type === 'page' && !/devtools/.test(t.url));
  if (!page) {
    log('no page target; nothing captured');
    process.exit(2);
  }

  const cdp = await connect(page.webSocketDebuggerUrl);
  log('websocket connected');

  await cdp.send('DOM.enable');
  await cdp.send('CSS.enable');
  await cdp.send('Page.enable');
  log('domains enabled');

  fs.mkdirSync(OUT, { recursive: true });

  const metrics = await cdp.send('Page.getLayoutMetrics');
  const viewport = metrics.cssLayoutViewport || metrics.layoutViewport;
  fs.writeFileSync(path.join(OUT, 'viewport.json'), JSON.stringify(viewport, null, 1));
  log(`viewport ${Math.round(viewport.clientWidth)}x${Math.round(viewport.clientHeight)}`);

  // Screenshots need a live compositor; allow skipping so a capture can still
  // be taken if that path is the one misbehaving.
  if (process.env.CDP_SHOT !== '0') {
    const shot = await cdp.send('Page.captureScreenshot', { format: 'png' }, 60000);
    fs.writeFileSync(path.join(OUT, 'screenshot.png'), Buffer.from(shot.data, 'base64'));
    log('screenshot written');
  }

  log('requesting DOM snapshot...');
  const snapshot = await cdp.send('DOMSnapshot.captureSnapshot', {
    computedStyles: STYLES,
    includePaintOrder: false,
    includeDOMRects: true,
  }, 120000);
  fs.writeFileSync(path.join(OUT, 'dom-snapshot.json'), JSON.stringify(snapshot));
  log('snapshot written');

  const entries = decode(snapshot);

  // Parent lookups happen in node-index space, guarded against cycles.
  const byNodeIndex = new Map(entries.map((entry) => [entry.nodeIndex, entry]));

  const shortClass = (entry) =>
    entry.className.trim().split(/\s+/).filter(Boolean).slice(0, 2).join('.');

  const ownName = (entry) =>
    entry.tag + (entry.id ? `#${entry.id}` : '') + (shortClass(entry) ? `.${shortClass(entry)}` : '');

  const pathOf = (entry) => {
    const parts = [];
    const seen = new Set();
    let cursor = entry;
    while (cursor && !seen.has(cursor.nodeIndex)) {
      seen.add(cursor.nodeIndex);
      parts.unshift(ownName(cursor));
      cursor = byNodeIndex.get(cursor.parentNodeIndex);
    }
    return parts.join(' > ');
  };

  const rendered = entries.filter((entry) => entry.bounds.w > 0 && entry.bounds.h > 0);

  fs.writeFileSync(
    path.join(OUT, 'layout.json'),
    JSON.stringify(rendered.map((entry) => ({
      path: pathOf(entry),
      box: entry.bounds,
      styles: entry.styles,
    })), null, 1)
  );
  log(`layout.json: ${rendered.length} rendered nodes of ${entries.length}`);

  // Largest first: when transcribing a layout into widgets, area is the best
  // proxy for "this is a region, not a decoration".
  const byArea = [...rendered].sort(
    (a, b) => b.bounds.w * b.bounds.h - a.bounds.w * a.bounds.h
  );

  fs.writeFileSync(
    path.join(OUT, 'boxes.txt'),
    byArea.map((entry) => {
      const { x, y, w, h } = entry.bounds;
      const s = entry.styles;
      return [
        `${Math.round(w)}x${Math.round(h)} @ ${Math.round(x)},${Math.round(y)}`.padEnd(24),
        `pad ${s['padding-top']}/${s['padding-right']}/${s['padding-bottom']}/${s['padding-left']}`.padEnd(30),
        `gap ${s.gap}`.padEnd(11),
        `font ${s['font-size']}/${s['font-weight']}`.padEnd(16),
        `bg ${s['background-color']}`.padEnd(26),
        pathOf(entry).slice(-120),
      ].join(' ');
    }).join('\n') + '\n'
  );

  // Indented tree of rendered boxes: the shape that has to become GtkBoxes.
  const children = new Map();
  for (const entry of rendered) {
    if (!children.has(entry.parentNodeIndex)) children.set(entry.parentNodeIndex, []);
    children.get(entry.parentNodeIndex).push(entry);
  }
  const treeLines = [];
  const walk = (entry, depth) => {
    if (depth > 16 || treeLines.length > 900) return;
    const { w, h } = entry.bounds;
    treeLines.push(
      `${'  '.repeat(depth)}${ownName(entry)}  ${Math.round(w)}x${Math.round(h)}` +
      `  [${entry.styles.display}${entry.styles.gap && entry.styles.gap !== 'normal' ? ` gap:${entry.styles.gap}` : ''}]`
    );
    for (const child of children.get(entry.nodeIndex) || []) walk(child, depth + 1);
  };
  const roots = rendered.filter((entry) => !byNodeIndex.has(entry.parentNodeIndex));
  for (const root of roots) walk(root, 0);
  fs.writeFileSync(path.join(OUT, 'tree.txt'), treeLines.join('\n') + '\n');

  console.log('\n=== regions of interest ===');
  const interesting = /(sidebar|nav|composer|input|tag|chip|main-content|welcome|brand|hero|workspace|permission|footer|disclaimer|segment|tab)/i;
  const seenPaths = new Set();
  for (const entry of byArea) {
    if (!shortClass(entry) || !interesting.test(entry.className)) continue;
    const key = shortClass(entry) + Math.round(entry.bounds.w);
    if (seenPaths.has(key)) continue;
    seenPaths.add(key);
    const { x, y, w, h } = entry.bounds;
    console.log(
      `${String(Math.round(w)).padStart(5)}x${String(Math.round(h)).padStart(4)} @ ` +
      `${String(Math.round(x)).padStart(5)},${String(Math.round(y)).padStart(4)}  ` +
      `pad ${entry.styles['padding-top']}/${entry.styles['padding-right']}/${entry.styles['padding-bottom']}/${entry.styles['padding-left']}  ` +
      `gap ${entry.styles.gap}  bg ${entry.styles['background-color']}  ` +
      ownName(entry)
    );
    if (seenPaths.size > 60) break;
  }

  console.log(`\nwrote: ${OUT}/{screenshot.png,dom-snapshot.json,layout.json,boxes.txt,tree.txt}`);
  process.exit(0);
}

main().catch((error) => {
  console.error('capture failed:', error.message);
  process.exit(1);
});
