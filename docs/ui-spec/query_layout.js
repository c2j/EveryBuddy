// Print a compact style table for chosen CSS classes out of a capture_dom.js
// layout.json.
//
// Two gotchas this tool exists to absorb:
//   * DOMSnapshot `bounds` come back at the device scale factor (2 on this
//     display), so they are divided by DPR to give CSS px.
//   * computed style lengths are NOT scaled — they are already CSS px. Dividing
//     those too silently halves every font size and padding.
//
// Rows that are visually identical (same box, colours, font, radius) collapse
// into one, because a class usually matches many DOM ancestors that all inherit
// the same appearance.
//
//   node query_layout.js <layout.json> <class> [class...]
//   ALL=1 node query_layout.js ...       keep every row
//   LARGEST=1 node query_layout.js ...   only the biggest box per class, which
//                                        is what you want for a container

const fs = require('fs');

const DPR = Number(process.env.DPR || 2);
const ALL = process.env.ALL === '1';
const LARGEST = process.env.LARGEST === '1';
const rows = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
const wanted = process.argv.slice(3);

const classTokens = (path) =>
  [...path.matchAll(/\.([A-Za-z0-9_-]+)/g)].map((match) => match[1]);

const px = (value) => {
  if (typeof value !== 'string') return value;
  const match = value.match(/^(-?[\d.]+)px$/);
  return match ? `${Number(Number(match[1]).toFixed(2))}px` : value;
};

const clip = (value, max) =>
  typeof value === 'string' && value.length > max ? `${value.slice(0, max)}…` : value;

for (const selector of wanted) {
  const hits = rows
    .filter((row) => classTokens(row.path).includes(selector))
    .map((row) => {
      const s = row.styles;
      const box = `${Number((row.box.w / DPR).toFixed(2))}×${Number((row.box.h / DPR).toFixed(2))}` +
        ` @${Number((row.box.x / DPR).toFixed(1))},${Number((row.box.y / DPR).toFixed(1))}`;
      const font = `${px(s['font-size'])}/${s['font-weight']}`;
      const radius = px(s['border-radius']);
      return {
        row,
        box,
        font,
        radius,
        signature: [box, s['background-color'], s.color, font, radius].join('|'),
      };
    });

  if (hits.length === 0) {
    console.log(`${selector.padEnd(32)} <no match>`);
    continue;
  }

  const unique = [];
  const seen = new Set();
  for (const hit of hits) {
    if (LARGEST) {
      if (!seen.has('pick') || hit.row.box.w * hit.row.box.h > unique[0].row.box.w * unique[0].row.box.h) {
        seen.add('pick');
        unique.length = 0;
        unique.push(hit);
      }
      continue;
    }
    if (!ALL && seen.has(hit.signature)) continue;
    seen.add(hit.signature);
    unique.push(hit);
  }

  for (const { row, box, font, radius } of unique) {
    const s = row.styles;
    const pad = ['padding-top', 'padding-right', 'padding-bottom', 'padding-left']
      .map((key) => px(s[key]))
      .join(' ');
    console.log(
      selector.padEnd(32),
      box.padEnd(13),
      `bg ${s['background-color']}`.padEnd(23),
      `fg ${s.color}`.padEnd(22),
      `f ${font}`.padEnd(16),
      `r ${radius}`.padEnd(11),
      `gap ${px(s.gap)}`.padEnd(11),
      `pad ${pad}`.padEnd(26),
      `sh ${clip(s['box-shadow'], 48)}`
    );
  }
}
