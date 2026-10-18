/**
 * demo-03: 接入 dsh 存量插件,实现"最小可与大模型对话"。
 *
 * 存量栈(全部原样装载,无改写):
 *   - @deepseek-ai/dsh-llm            LlmRuntime(`ctx.llm`)、消息词汇、BlockAssembler
 *   - @deepseek-ai/dsh-llm-deepseek    DeepSeek 协议适配器(registerDeepSeekProvider)
 *   - @deepseek-ai/dsh-system-prompt   真实 system prompt 服务
 *   - @deepseek-ai/dsh-tools           真 ToolRuntime(`ctx.tools`)
 *   - @deepseek-ai/dsh-tool-fs         真 read 工具 —— ctx.fs 由 Rust 内核兑现
 *
 * 演示自身的胶水只有两块,并且如实标注:
 *   - `settings` 桩:适配器只调用 `configure` 一个方法;真实部署挂
 *     @deepseek-ai/dsh-settings 全家(configEditor/profileContext)。
 *   - 凭据:直接读 DEEPSEEK_API_KEY;真实部署走 llm-deepseek-api-key 插件
 *     (credentials 服务 + launch-environment)。
 *
 * 循环本体使用 dsh 的消息词汇与装配器:createUserMessage → llm.stream →
 * BlockAssembler → 工具调用块 → def.execute(Rust fs)→ output.render →
 * createToolResultMessage → 下一轮。
 *
 *   DEEPSEEK_API_KEY=... node demo-03-chat.mjs              # 交互多轮
 *   DEEPSEEK_API_KEY=... node demo-03-chat.mjs "问题..."    # 单轮
 *   EVO_MODEL=deepseek-reasoner node demo-03-chat.mjs       # 换模型
 */
import assert from 'node:assert/strict'
import path from 'node:path'
import readline from 'node:readline/promises'
import { fileURLToPath } from 'node:url'

import { LlmRuntime, BlockAssembler, createUserMessage, createToolResultMessage } from '../../packages/llm/llm/lib/index.js'
import { registerDeepSeekProvider, resolveAdapterOptions } from '../../packages/llm/llm-deepseek/lib/index.js'
import { FsError } from '../../packages/fs/fs/lib/index.js'

import { startCoKernel } from './src/co-kernel.js'
import { WasmKernel } from './src/wasm-kernel.js'

const here = path.dirname(fileURLToPath(import.meta.url))
const repoRoot = path.join(here, '..', '..')
const modulePath = path.join(here, '..', 'kernel', 'target', 'wasm32-wasip1', 'release', 'evobuddy_kernel.wasm')

const PROVIDER = 'deepseek-official'
const MODEL = process.env.EVO_MODEL ?? 'deepseek-chat'
const MAX_TOOL_ROUNDS = 6

const apiKey = process.env.DEEPSEEK_API_KEY
if (!apiKey || apiKey.trim().length === 0) {
  console.log('demo-03 SKIP:未设置 DEEPSEEK_API_KEY(与仓库 e2e 同一约定;设置后重跑)')
  process.exit(0)
}

// ── 内核:fs 能力,挂载整个仓库(模型能读到什么,由 preopen 划定)──────────
const mountRoot = process.env.EVO_ROOT ?? repoRoot
const kernel = await WasmKernel.load({
  modulePath,
  preopens: { '/workspace': mountRoot },
  onLog: (entry) => console.log(`  [core] ${entry.message}`),
})
kernel.boot({ root: '/workspace', patterns: ['evobuddy'] })

// ── 共存内核:存量 dsh 服务 + 两个如实标注的演示接缝 ─────────────────────
const options = () => resolveAdapterOptions(
  process.env.DEEPSEEK_BASE_URL ? { baseURL: process.env.DEEPSEEK_BASE_URL } : {},
  undefined,
)
const connection = options()

const co = await startCoKernel({
  kernel,
  // settings 桩:适配器装载时唯一调用的方法。真实部署挂 dsh-settings 全家。
  mount: (ctx) => ctx.reflect.provide('settings', { configure() {} }),
})

await co.load(LlmRuntime)
await co.load({
  name: 'llm-deepseek-demo-glue',
  inject: ['llm'],
  apply(ctx) {
    registerDeepSeekProvider(ctx, PROVIDER, {
      options,
      providerName: 'DeepSeek',
      resolveAuth: async () => ({ headers: { 'x-api-key': apiKey } }),
      discoverModels: async () => [],
    })
  },
})
const toolFs = await import(path.join(repoRoot, 'packages', 'fs', 'tool-fs', 'lib', 'index.js'))
await co.load(toolFs, { read: true, readImage: false, write: false, edit: false })

const read = co.ctx.tools.get('read')
assert.ok(read, 'read 工具已在真 ToolRuntime 注册')
const toolDefs = new Map([[read.name, read]])
const toolSchemas = [...toolDefs.values()].map((def) => ({
  name: def.name,
  description: def.description,
  parameters: def.parameters,
}))

console.log('demo-03:最小大模型对话(dsh 存量栈 + Rust 内核 fs)')
console.log(`  端点: ${connection.baseURL}  模型: ${MODEL}  路由: ${PROVIDER}`)
console.log(`  工具: ${[...toolDefs.keys()].join(', ')}(执行体 = Rust 内核的 fs)`)
console.log(`  挂载: ${mountRoot} → /workspace`)
console.log('  多轮对话,/exit 或 Ctrl-D 退出\n')

// ── 对话循环:dsh 消息词汇 + BlockAssembler ─────────────────────────────
const messages = []
const SYSTEM_PROMPT = [
  '你是 EvoBuddy 演示助手,运行在 DeepSeek Harness 的双内核原型上:',
  '文件读取能力(ctx.fs)由 Rust wasm 内核通过 WASI 兑现,而不是 Node。',
  `你可以用 read 工具读取挂载在 /workspace 的代码树(当前是本仓库)。`,
  '回答用中文;引用文件时给出路径。读文件前先想清楚要读的相对路径。',
].join('\n')

let toolRoundsTotal = 0

async function runToolCall(call) {
  const def = toolDefs.get(call.name)
  if (!def) {
    return { content: [{ type: 'text', text: `no such tool: ${call.name}` }], isError: true }
  }
  let args
  try {
    args = JSON.parse(call.arguments || '{}')
  } catch {
    return { content: [{ type: 'text', text: `tool arguments are not valid JSON: ${call.arguments}` }], isError: true }
  }
  const exec = { callId: call.id, name: call.name, arguments: args, signal: new AbortController().signal }
  try {
    const value = await def.execute(args, exec)
    return { content: def.output.render(args, value), isError: false }
  } catch (error) {
    const message = error instanceof FsError || error instanceof Error ? error.message : String(error)
    return { content: [{ type: 'text', text: message }], isError: true }
  }
}

async function chatTurn(input) {
  messages.push(createUserMessage({ content: [{ type: 'text', text: input }] }))

  for (let round = 0; round < MAX_TOOL_ROUNDS; round += 1) {
    const assembler = new BlockAssembler()
    const stream = co.ctx.llm.stream({
      provider: PROVIDER,
      model: MODEL,
      messages,
      system: SYSTEM_PROMPT,
      tools: toolSchemas,
    })
    for await (const chunk of stream) {
      if (chunk.type === 'text-delta') process.stdout.write(chunk.text)
      assembler.push(chunk)
    }
    const finish = assembler.finish
    if (finish && (finish.kind === 'error' || finish.kind === 'aborted')) {
      const failure = finish.failure
      throw new Error(`模型流失败 [${failure?.code ?? '?'}]: ${failure?.message ?? finish.kind}`)
    }

    const assistant = assembler.message({ provider: PROVIDER, model: MODEL })
    messages.push(assistant)
    console.log()

    const calls = assistant.content.filter((block) => block.type === 'tool-call')
    if (calls.length === 0) {
      if (assembler.usage) {
        console.log(`  (tokens: ${assembler.usage.inputTokens ?? '?'} 入 / ${assembler.usage.outputTokens ?? '?'} 出)`)
      }
      return
    }

    for (const call of calls) {
      console.log(`  ⚙ ${call.name}(${call.arguments.slice(0, 140)})`)
      const result = await runToolCall(call)
      const head = result.content
        .map((block) => block.text ?? JSON.stringify(block))
        .join(' ')
        .split('\n').slice(0, 3).join(' ⏎ ')
      console.log(`  ${result.isError ? '✗' : '✓'} ${head.slice(0, 200)}`)
      messages.push(createToolResultMessage({
        callId: call.id,
        content: result.content,
        isError: result.isError,
      }))
      toolRoundsTotal += 1
    }
  }
  console.log(`\n  (已达单轮工具上限 ${MAX_TOOL_ROUNDS},停止本轮)`)
}

// ── 单轮或交互 ────────────────────────────────────────────────────────
const oneShot = process.argv.slice(2).filter((arg) => !arg.startsWith('-')).join(' ')
if (oneShot.length > 0) {
  await chatTurn(oneShot)
  console.log(`\n(本轮共 ${toolRoundsTotal} 次工具调用,全部经 Rust 内核 fs 兑现)`)
  kernel.shutdown()
  process.exit(0)
}

const rl = readline.createInterface({ input: process.stdin, output: process.stdout })
try {
  for (;;) {
    let input
    try {
      input = (await rl.question('你> ')).trim()
    } catch {
      break // stdin 已关闭(管道 EOF / Ctrl-D)
    }
    if (input.length === 0) continue
    if (['/exit', '/quit', 'exit', 'quit'].includes(input)) break
    try {
      await chatTurn(input)
    } catch (error) {
      console.log(`  ✗ ${error instanceof Error ? error.message : String(error)}`)
    }
  }
} finally {
  rl.close()
  kernel.shutdown()
  console.log('\ndemo-03 结束')
}
