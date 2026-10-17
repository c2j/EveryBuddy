// Print a grid of pixels from a PNG.
//
// Written because the image tools on this machine cannot crop or sample a
// region, and "is that stripe a stray element, an uninitialised surface or a
// font difference?" is a question about specific pixels.
//
//   node pngpx.js <file.png> [rows] [cols]
const fs = require('fs');
const zlib = require('zlib');

function decode(file) {
  const buf = fs.readFileSync(file);
  if (buf.readUInt32BE(0) !== 0x89504e47) throw new Error('not a png');
  let pos = 8, width = 0, height = 0, depth = 0, color = 0;
  const idat = [];
  while (pos < buf.length) {
    const len = buf.readUInt32BE(pos);
    const type = buf.toString('ascii', pos + 4, pos + 8);
    const data = buf.subarray(pos + 8, pos + 8 + len);
    if (type === 'IHDR') {
      width = data.readUInt32BE(0);
      height = data.readUInt32BE(4);
      depth = data[8];
      color = data[9];
      if (data[12] !== 0) throw new Error('interlaced png');
    } else if (type === 'IDAT') idat.push(data);
    else if (type === 'IEND') break;
    pos += 12 + len;
  }
  const channels = { 0: 1, 2: 3, 3: 1, 4: 2, 6: 4 }[color];
  const bpp = channels * (depth / 8);
  const raw = zlib.inflateSync(Buffer.concat(idat));
  const stride = width * bpp;
  const out = Buffer.alloc(height * stride);
  let ri = 0;
  for (let y = 0; y < height; y++) {
    const filter = raw[ri++];
    const line = raw.subarray(ri, ri + stride);
    ri += stride;
    const cur = out.subarray(y * stride, (y + 1) * stride);
    const prev = y ? out.subarray((y - 1) * stride, y * stride) : Buffer.alloc(stride);
    for (let x = 0; x < stride; x++) {
      const a = x >= bpp ? cur[x - bpp] : 0;
      const b = prev[x];
      const c = x >= bpp ? prev[x - bpp] : 0;
      let v = line[x];
      if (filter === 1) v = (v + a) & 255;
      else if (filter === 2) v = (v + b) & 255;
      else if (filter === 3) v = (v + ((a + b) >> 1)) & 255;
      else if (filter === 4) {
        const p = a + b - c;
        const pa = Math.abs(p - a), pb = Math.abs(p - b), pc = Math.abs(p - c);
        v = (v + (pa <= pb && pa <= pc ? a : pb <= pc ? b : c)) & 255;
      } else if (filter !== 0) throw new Error('filter ' + filter);
      cur[x] = v;
    }
  }
  return { width, height, bpp, color, depth, px: out };
}

const file = process.argv[2];
const { width, height, bpp, px, color, depth } = decode(file);
console.log(`${file}: ${width}x${height}, colortype ${color}, depth ${depth}`);
const at = (x, y) => {
  const o = (y * width + x) * bpp;
  return color === 2 || color === 6 ? [px[o], px[o + 1], px[o + 2]] : [px[o], px[o], px[o]];
};
const rows = (process.argv[3] || '0,10,40,60,80,96,99,100,120,200,300,500,700,860').split(',').map(Number);
const cols = (process.argv[4] || '0,1,2,3,4,6,10,50,150,279,281').split(',').map(Number);
console.log('y\\x ' + cols.map((c) => String(c).padStart(12)).join(''));
for (const y of rows) {
  console.log(
    String(y).padEnd(4) +
      cols.map((c) => at(c, y).join(',').padStart(12)).join(''),
  );
}
