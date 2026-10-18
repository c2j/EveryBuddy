//! EvoBuddy 最小大模型对话 —— 完全原生的 Rust 进程。
//!
//! 与 demo-03(Node 侧,接 dsh 存量 JS 栈)相对:这一份里**没有 Node、没有
//! wasm、没有 JS**。同一份内核(原生 rlib)提供 fs/检索/策略闸门;对话循环、
//! Anthropic Messages 线协议、工具执行全部是原生 Rust。它证明的是架构的另一半:
//! 内核不依附 JS 宿主也能构成一个完整的智能体产品;Node/dsh 是它的一个消费方,
//! 不是前提。
//!
//! 工具经内核的**请求面**执行(fs.resolve / fs.readText / search.find)——
//! 与 wasm 宿主走的是同一套方法与同一个策略闸门。
//!
//!   DEEPSEEK_API_KEY=… cargo run --release -- --root ../..
//!   DEEPSEEK_API_KEY=… cargo run --release -- --root ../.. --ask "一句话问题"
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use anthropic::{Client, Delta};
use evobuddy_kernel::kernel::Kernel;
use evobuddy_kernel::protocol::{Request, Response};
use evobuddy_kernel::search::SearchLimits;
use serde_json::{Value, json};

mod anthropic;

/// 单次用户输入允许的工具轮数(与 demo-03 同一上限)。
const MAX_TOOL_ROUNDS: usize = 6;
/// Anthropic 必填的输出上限。
const MAX_TOKENS: u64 = 8192;

fn main() -> Result<(), String> {
    let api_key = match std::env::var("DEEPSEEK_API_KEY") {
        Ok(key) if !key.trim().is_empty() => key,
        _ => {
            println!("evobuddy-chat SKIP:未设置 DEEPSEEK_API_KEY(与仓库 e2e 同一约定;设置后重跑)");
            return Ok(());
        }
    };
    let model = std::env::var("EVO_MODEL").unwrap_or_else(|_| "deepseek-chat".to_string());

    let mut root = None;
    let mut ask: Option<String> = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--root" => root = args.next().map(PathBuf::from),
            "--ask" => ask = Some(args.next().unwrap_or_default()),
            other => return Err(format!("未知参数 {other:?};用法: [--root <目录>] [--ask <问题>]")),
        }
    }
    let root = root
        .map(|r| r.canonicalize().unwrap_or(r))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    println!("== EvoBuddy 对话(原生 Rust,无 Node / 无 wasm / 无 JS)==");
    println!("挂载根: {}", root.display());
    println!("模型:   {model} @ DeepSeek(Anthropic Messages 协议)");
    println!("工具:   read, search(执行体 = 内核请求面;策略闸门在链上)\n");

    let mut kernel = Kernel::boot(
        root.clone(),
        vec!["evobuddy".to_string()],
        SearchLimits::default(),
    )?;
    let client = Client::new(
        std::env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com/anthropic".to_string()),
        api_key,
        model.clone(),
        MAX_TOKENS,
    );

    let system = format!(
        "你是 EvoBuddy 演示助手,运行在一个 Rust 内核原型上:文件读取与内容检索\n\
         由 Rust 内核(原生进程)兑现。挂载根是 {},工具的 file_path 用相对路径。\n\
         回答用中文;引用文件时给出路径;需要事实依据时先调用工具,不要凭空猜测。",
        root.display(),
    );
    let tools = tools_json();

    let mut history: Vec<Value> = Vec::new();
    let mut next_id = AtomicU64::new(1000);

    let mut run_input = |history: &mut Vec<Value>, input: &str, kernel: &mut Kernel| -> Result<(), String> {
        history.push(anthropic::user_message(input));
        for _round in 0..MAX_TOOL_ROUNDS {
            let turn = stream_turn(&client, &system, history, &tools)?;
            history.push(anthropic::assistant_message(turn.content));

            if turn.tool_calls.is_empty() {
                println!(
                    "\n  (tokens: {} 入 / {} 出)",
                    turn.input_tokens, turn.output_tokens,
                );
                if turn.stop_reason == "max_tokens" {
                    println!("  (输出被 max_tokens 截断)");
                }
                return Ok(());
            }

            let mut results: Vec<Value> = Vec::new();
            for call in &turn.tool_calls {
                println!("  ⚙ {}({})", call.name, shorten(&call.arguments, 140));
                let args: Value = serde_json::from_str(&call.arguments).unwrap_or(json!({}));
                let (text, is_error) = run_tool(kernel, &mut next_id, &call.name, &args);
                let head: String = text.lines().take(3).collect::<Vec<_>>().join(" ⏎ ");
                println!("  {} {}", if is_error { "✗" } else { "✓" }, shorten(&head, 200));
                results.push(anthropic::tool_result_block(&call.id, &text, is_error));
            }
            // 一个 assistant 帧的全部 tool_use,必须由紧接着的一帧 user 带齐。
            history.push(json!({ "role": "user", "content": results }));
        }
        println!("\n  (已达单轮工具上限 {MAX_TOOL_ROUNDS},停止本轮)");
        Ok(())
    };

    // 单轮模式。
    if let Some(question) = ask.filter(|q| !q.trim().is_empty()) {
        run_input(&mut history, &question, &mut kernel)?;
        kernel.shutdown();
        return Ok(());
    }

    // 交互模式。
    println!("多轮对话,/exit 或 Ctrl-D 退出。\n");
    let stdin = std::io::stdin();
    loop {
        print!("你> ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            println!();
            break;
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if ["/exit", "/quit", "exit", "quit"].contains(&input) {
            break;
        }
        if let Err(error) = run_input(&mut history, input, &mut kernel) {
            println!("  ✗ {error}");
        }
    }
    kernel.shutdown();
    println!("内核已关闭");
    Ok(())
}

/// 一次流式回复;文本即时回显,思考增量灰化提示一轮。
fn stream_turn(
    client: &Client,
    system: &str,
    history: &[Value],
    tools: &Value,
) -> Result<anthropic::Turn, String> {
    let messages = Value::Array(history.to_vec());
    let mut printed_thinking = false;
    let mut any_text = false;
    let turn = client.stream(system, &messages, tools, &mut |delta: &Delta| match delta {
        Delta::Text(text) => {
            any_text = true;
            print!("{text}");
            std::io::stdout().flush().ok();
        }
        Delta::Thinking(text) => {
            if !printed_thinking {
                print!("\x1b[2m(思考) ");
                printed_thinking = true;
            }
            print!("{text}");
            std::io::stdout().flush().ok();
        }
    })?;
    if printed_thinking {
        print!("\x1b[0m");
    }
    if any_text || printed_thinking {
        println!();
    }
    Ok(turn)
}

fn shorten(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out.replace('\n', " ⏎ ")
}

/// 工具面:与 wasm 宿主同一个请求面,同一个策略闸门。
fn run_tool(kernel: &mut Kernel, next_id: &mut AtomicU64, name: &str, args: &Value) -> (String, bool) {
    // 每个请求一个新 id;内核会原样回显,供宿主配对。
    macro_rules! call {
        ($method:expr, $params:expr) => {{
            let id = next_id.fetch_add(1, Ordering::Relaxed);
            kernel.handle(Request { id, method: $method.to_string(), params: $params })
        }};
    }
    match name {
        "read" => {
            let Some(path) = args["file_path"].as_str() else {
                return ("缺少 file_path".into(), true);
            };
            let resolved = call!("fs.resolve", json!({ "path": path }));
            if !resolved.ok {
                return (describe_failure(&resolved), true);
            }
            let target = resolved.result.unwrap()["targetKey"].as_str().unwrap_or_default().to_string();
            let read = call!("fs.readText", json!({ "targetKey": target }));
            if !read.ok {
                return (describe_failure(&read), true);
            }
            let content = read.result.unwrap()["content"].as_str().unwrap_or_default().to_string();
            let offset = args["offset"].as_u64().unwrap_or(1).max(1) as usize;
            let limit = args["limit"].as_u64().unwrap_or(40).max(1) as usize;
            let total = content.lines().count();
            let mut out = format!("共 {total} 行;第 {offset} 行起:\n");
            for (index, line) in content.lines().enumerate().skip(offset - 1).take(limit) {
                let line: String = line.chars().take(200).collect();
                out.push_str(&format!("{:>5}  {line}\n", index + 1));
            }
            (out, false)
        }
        "search" => {
            let patterns: Vec<String> = match args.get("patterns").or_else(|| args.get("pattern")) {
                Some(list) if list.is_array() => list
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|v| v.as_str())
                    .map(str::to_string)
                    .collect(),
                Some(single) if single.is_string() => vec![single.as_str().unwrap().to_string()],
                _ => return ("缺少 patterns(数组)或 pattern(字符串)".into(), true),
            };
            // 模式活在 provider 插件里:换模式 = 一次热插拔(provider 换代,
            // 依赖它的 sweep 插件自动重扫),然后才查询。find 的参数里没有
            // patterns——这是内核的架构裁决,不是遗漏。
            call!("plugins.reload", json!({ "patterns": patterns }));
            let mut params = json!({});
            if let Some(root) = args["root"].as_str() {
                params["root"] = json!(root);
            }
            let found = call!("search.find", params);
            if !found.ok {
                return (describe_failure(&found), true);
            }
            let outcome = found.result.unwrap();
            let mut out = format!(
                "{} 处匹配 / {} 个文件 / {} ms{}\n",
                outcome["totalMatches"],
                outcome["filesScanned"],
                outcome["elapsedMs"],
                if outcome["truncated"].as_bool().unwrap_or(false) {
                    "(遍历被上限截断,只覆盖了按路径排序的前一段;收窄 patterns 或指定更小的 root 再试)"
                } else {
                    ""
                },
            );
            if let Some(hits) = outcome["hits"].as_array() {
                for hit in hits.iter().take(20) {
                    out.push_str(&format!(
                        "  {}:{}:{}  [{}] {}\n",
                        hit["path"], hit["line"], hit["column"], hit["pattern"], hit["text"],
                    ));
                }
                let rest = hits.len().saturating_sub(20);
                if rest > 0 {
                    out.push_str(&format!("  … 其余 {rest} 条略\n"));
                }
            }
            (out, false)
        }
        other => (format!("没有工具 {other}"), true),
    }
}

/// 内核拒绝 → 给模型的文本(保留稳定码,模型能据此改参数)。
fn describe_failure(response: &Response) -> String {
    match &response.error {
        Some(error) => format!("{}: {}", error.code, error.message),
        None => "unknown failure".to_string(),
    }
}

/// 两个工具的声明:语义与 dsh read / 内核检索一致。
fn tools_json() -> Value {
    json!([
        {
            "name": "read",
            "description": "读取挂载树里的一个 UTF-8 文本文件,返回行号窗口。",
            "input_schema": {
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "相对挂载根的路径" },
                    "offset": { "type": "integer", "description": "1 起始行号,默认 1" },
                    "limit": { "type": "integer", "description": "最多返回的行数,默认 40" },
                },
                "required": ["file_path"],
            },
        },
        {
            "name": "search",
            "description": "在挂载树上做一次多模式内容检索(进程内 aho-corasick,单遍)。可用它先定位,再用 read 精读。root 是相对挂载根的路径,可省略。",
            "input_schema": {
                "type": "object",
                "properties": {
                    "patterns": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "要匹配的字面字符串,一次可给多个",
                    },
                    "root": { "type": "string", "description": "可选;缺省为挂载根" },
                },
                "required": ["patterns"],
            },
        },
    ])
}
