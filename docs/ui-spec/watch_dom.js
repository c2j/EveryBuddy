// Repeatedly snapshot WorkBuddy's live layout while a human drives the app, so
// that transient states (the chat view, open popovers, menus) are recorded even
// though each only exists for a moment.
//
// Each round shells out to capture_dom.js, so the parsing and the file formats
// stay in exactly one place. Kept frames land in <out>/frame-NNN/.
//
//   node watch_dom.js <out-dir>              (CDP_PORT env, default 9333)
//   WATCH_INTERVAL_MS=4000                   gap between frames
//   WATCH_LIMIT=300                          stop after N rounds
//   WATCH_HEARTBEAT=30                       keep a frame every N rounds even
//                                            if nothing changed
//
// A frame is kept when the *set of DOM classes* on screen differs from the last
// kept frame, so a screen the human leaves open for a minute costs one frame
// rather than fifteen. Stop early by creating <out-dir>/STOP.
//
// Read-only, exactly like capture_dom.js. The output contains the user's own
// workspace content, so it must go to a temp directory, never the repo.

const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');

const OUT = process.argv[2];
if (!OUT) {
  console.error('usage: node watch_dom.js <out-dir>');
  process.exit(1);
}

const INTERVAL = Number(process.env.WATCH_INTERVAL_MS || 4000);
const LIMIT = Number(process.env.WATCH_LIMIT || 300);
const HEARTBEAT = Number(process.env.WATCH_HEARTBEAT || 30);
const CAPTURE = path.join(__dirname, 'capture_dom.js');
const SERIAL = path.join(OUT, 'frames.txt');
const STOP = path.join(OUT, 'STOP');

fs.mkdirSync(OUT, { recursive: true });
fs.rmSync(STOP, { force: true });

/** Synchronous sleep; there is no timer to await in a straight-line loop. */
const sleep = (ms) => { Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms); };

const stamp = () => new Date().toISOString().replace('T', ' ').slice(0, 19);

/**
 * A stable fingerprint of "what is on screen": every class name mentioned in
 * the region report, sorted. Sizes and coordinates are dropped, so a frame is
 * only new when the set of visible components actually changes.
 */
const signatureOf = (regions) => {
  const names = new Set();
  for (const match of regions.matchAll(/\b[a-z][a-z0-9-]*\.[A-Za-z0-9_-]+/g)) {
    names.add(match[0]);
  }
  return [...names].sort().join(' ');
};

/** Regions that only appear once a task is running, used to label frames. */
const CHAT_MARKERS = /(conversation|message|chat-|composer|agent|tool-|reasoning|thread|bubble|turn)/i;

let kept = 0;
let lastSignature = null;

for (let i = 0; i < LIMIT; i++) {
  if (fs.existsSync(STOP)) {
    console.error(`[watch] STOP seen after ${i} round(s), ${kept} frame(s) kept`);
    break;
  }

  const name = `frame-${String(kept).padStart(3, '0')}`;
  const dir = path.join(OUT, name);
  let regions;

  try {
    regions = execFileSync(process.execPath, [CAPTURE, dir], {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, CDP_SHOT: '1' },
    });
  } catch (error) {
    // A failed round is not fatal: the whole point of watching is that the app
    // is in flux (restarting, no page target, mid-navigation).
    fs.rmSync(dir, { recursive: true, force: true });
    console.error(`[watch] round ${i} failed: ${String(error.message).split('\n')[0]}`);
    lastSignature = null; // force the next good round to be kept
    sleep(INTERVAL);
    continue;
  }

  const signature = signatureOf(regions);
  const changed = signature !== lastSignature;

  if (changed || i % HEARTBEAT === 0) {
    fs.writeFileSync(path.join(dir, 'regions.txt'), regions);
    fs.writeFileSync(path.join(dir, 'signature.txt'), `${signature}\n`);
    const chat = CHAT_MARKERS.test(regions);
    fs.appendFileSync(SERIAL, `${name} ${stamp()} round=${i} chat=${chat ? 'yes' : 'no'}\n`);
    console.error(`[watch] kept ${name} (round ${i}, chat=${chat ? 'yes' : 'no'})`);
    lastSignature = signature;
    kept++;
  } else {
    fs.rmSync(dir, { recursive: true, force: true });
  }

  sleep(INTERVAL);
}

console.error(`[watch] done, ${kept} frame(s) in ${OUT}`);
