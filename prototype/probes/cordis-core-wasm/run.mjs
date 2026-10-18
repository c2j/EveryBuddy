/**
 * Hosts the cordis-core-on-wasm probe.
 *
 * Unlike the rutis probe this module needs no WASI: it targets
 * `wasm32-unknown-unknown` and its only host import is `env.host_log`.
 * It must be built with `RUSTFLAGS='--cfg tokio_unstable'`.
 *
 * Usage: node run.mjs
 */
import fs from 'node:fs'

const bytes = fs.readFileSync('target/wasm32-unknown-unknown/release/ccore.wasm')

let instance

const { instance: started } = await WebAssembly.instantiate(bytes, {
  env: {
    host_log: (ptr, len) => {
      const view = new Uint8Array(instance.exports.memory.buffer, ptr, len)
      process.stdout.write(Buffer.from(view).toString('utf8') + '\n')
    },
  },
})
instance = started

const responsePtr = instance.exports.boot()
const length = new DataView(instance.exports.memory.buffer).getUint32(responsePtr, true)
const body = Buffer.from(
  new Uint8Array(instance.exports.memory.buffer, responsePtr + 4, length),
).toString('utf8')

console.log(`  [wasm -> host] result: ${body}`)
