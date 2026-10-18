//! 从 Rust 侧驱动内核:原生进程,无 wasm、无 Node。
//!
//! `Kernel` 及其下的检索、文件系统、策略链都是普通 Rust 库代码;`abi.rs` 的
//! wasm 导出只是它们上面的一层薄壳(见 `lib.rs` 的 `cfg` 门控)。三种模式:
//!
//! - `tour [root] [pattern...]` — 脚本化巡礼:启动、检索、策略否决、热插拔、fs
//! - `shell [root]` — 交互式外壳:现场敲命令驱动同一个内核
//! - `bench [root] [pattern...]` — 全树单遍多模式基准(files/MB/s)
//!
//! 与 wasm 宿主的差异只有两处,都是诚实的:日志走原生 transport(stderr);
//! 受限只由 `FsService` 的前缀检查执行(WASI 的能力层在原生目标上不存在),
//! 所以原生看到的是单层防御,wasm 里是双层。
//!
//! 用法:`cargo run --release -- <tour|shell|bench> [root] [pattern...]`
//! (无子命令时保持旧行为:对 `.` 跑 tour。)
use std::io::{BufRead, Write};
use std::path::PathBuf;

use evobuddy_kernel::kernel::{Kernel, PluginState};
use evobuddy_kernel::protocol::{Request, Response};
use evobuddy_kernel::search::SearchLimits;

/// 吞吐基准的检索上限:计数要真实,上限就不能先于遍历到达。
const BENCH_MAX_MATCHES: usize = 50_000_000;

fn request(id: u64, method: &str, params: serde_json::Value) -> Request {
    Request {
        id,
        method: method.to_string(),
        params,
    }
}

/// 发一个请求,打印结果,返回 result 供后续步骤引用。
fn show(step: &str, response: Response) -> serde_json::Value {
    if response.ok {
        println!("  [{step}] ok");
        return response.result.unwrap_or(serde_json::Value::Null);
    }
    match response.error {
        Some(error) => println!("  [{step}] {} : {}", error.code, error.message),
        None => println!("  [{step}] empty reply"),
    }
    serde_json::Value::Null
}

fn strings(value: &serde_json::Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .map(|list| list.iter().filter_map(|v| v.as_str()).map(str::to_string).collect())
        .unwrap_or_default()
}

/// 打印一次检索结果;`max_hits` 截断展示(计数仍展示总量)。
fn print_outcome(outcome: &serde_json::Value, max_hits: usize) {
    println!(
        "        {} 处匹配 / {} 个文件 / {} ms(扫描 {} MB)",
        outcome["totalMatches"],
        outcome["filesScanned"],
        outcome["elapsedMs"],
        outcome["bytesScanned"].as_u64().unwrap_or(0) / (1024 * 1024),
    );
    if let Some(hits) = outcome["hits"].as_array() {
        for hit in hits.iter().take(max_hits) {
            println!(
                "          {}:{}:{}  [{}] {}",
                hit["path"], hit["line"], hit["column"], hit["pattern"], hit["text"],
            );
        }
        let rest = hits.len().saturating_sub(max_hits);
        if rest > 0 {
            println!("          … 其余 {rest} 条略");
        }
    }
}

fn states_line(states: &[PluginState]) -> String {
    states
        .iter()
        .map(|s| format!("{}@{}", s.name, s.id))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Shell 参数按空白切分;成对引号只当定界符,不是模式的一部分。
fn strip_quotes(word: &str) -> String {
    let trimmed = word.trim();
    if trimmed.len() >= 2
        && ((trimmed.starts_with('"') && trimmed.ends_with('"'))
            || (trimmed.starts_with('\'') && trimmed.ends_with('\'')))
    {
        trimmed[1..trimmed.len() - 1].to_string()
    } else {
        trimmed.to_string()
    }
}

/// 大数毫秒只保留一位小数,报告才可读。
fn round_ms(ms: f64) -> String {
    format!("{ms:.1}")
}

/// 脚本化巡礼:一次看完全部能力面。
fn tour(mut kernel: Kernel, root: &PathBuf) -> Result<(), String> {
    println!("== EvoBuddy 内核:Rust 侧启动(原生,无 wasm、无 Node)==");
    println!("root: {}", root.display());
    println!("plugins: {}", states_line(&kernel.states()));

    let hello = show("kernel.hello", kernel.handle(request(1, "kernel.hello", serde_json::json!({}))));
    println!(
        "        runtime={} version={}, 方法 {} 个",
        hello["runtime"].as_str().unwrap_or("?"),
        hello["version"].as_str().unwrap_or("?"),
        hello["methods"].as_array().map(Vec::len).unwrap_or_default(),
    );

    let found = show(
        "search.find",
        kernel.handle(request(2, "search.find", serde_json::json!({}))),
    );
    print_outcome(&found, 5);

    // 策略闸门:挂载点之外的根被 waterfall 否决,检索根本不会开始。
    let outside = root
        .parent()
        .map(|p| p.display().to_string())
        .filter(|p| !p.is_empty())
        .unwrap_or_else(|| "/".to_string());
    show(
        "search.find(逃逸)",
        kernel.handle(request(3, "search.find", serde_json::json!({ "root": outside }))),
    );

    // 热插拔:换模式 = 换 provider fiber,consumer 依赖重启再扫一遍。
    let before = kernel.states();
    let swapped: Vec<String> = vec!["impl ".to_string(), "struct ".to_string()];
    show(
        "plugins.reload",
        kernel.handle(request(4, "plugins.reload", serde_json::json!({ "patterns": swapped }))),
    );
    let after = kernel.states();
    println!(
        "        fibers: {} -> {}(同一组角色,新的实例)",
        states_line(&before),
        states_line(&after),
    );
    let again = show(
        "search.find",
        kernel.handle(request(5, "search.find", serde_json::json!({}))),
    );
    println!("        换模式后: {} 处匹配", again["totalMatches"]);

    let reports = show(
        "search.reports",
        kernel.handle(request(6, "search.reports", serde_json::json!({}))),
    );
    if let Some(reports) = reports.as_array() {
        for (generation, report) in reports.iter().enumerate() {
            println!(
                "        第{}代: {:?} -> {} 处 / {} 文件",
                generation + 1,
                strings(report, "patterns"),
                report["totalMatches"],
                report["filesScanned"],
            );
        }
    }

    // 文件系统:解析并读取一个真实文件。
    let target = show(
        "fs.resolve",
        kernel.handle(request(7, "fs.resolve", serde_json::json!({ "path": "Cargo.toml" }))),
    );
    let read = kernel.handle(request(
        8,
        "fs.readText",
        serde_json::json!({ "targetKey": target["targetKey"] }),
    ));
    let content = read
        .result
        .as_ref()
        .and_then(|r| r["content"].as_str())
        .map(str::to_string);
    match content {
        Some(content) => {
            let first = content.lines().next().unwrap_or("");
            println!(
                "  [fs.readText] Cargo.toml 共 {} 字节,首行: {first}",
                content.len()
            );
        }
        None => {
            show("fs.readText", read);
        }
    }

    kernel.shutdown();
    println!("== 内核已关闭;交互式:`cargo run --release -- shell` ==");
    Ok(())
}

/// 行号窗口读文件,与 dsh read 工具同一套 offset/limit 语义。
fn print_read_window(
    kernel: &mut Kernel,
    id: &mut u64,
    path: &str,
    offset: usize,
    limit: usize,
) {
    *id += 1;
    let resolved = show(
        "fs.resolve",
        kernel.handle(request(*id, "fs.resolve", serde_json::json!({ "path": path }))),
    );
    if resolved["targetKey"].is_null() {
        return;
    }
    *id += 1;
    let read = kernel.handle(request(
        *id,
        "fs.readText",
        serde_json::json!({ "targetKey": resolved["targetKey"] }),
    ));
    if !read.ok {
        show("fs.readText", read);
        return;
    }
    let content = read
        .result
        .as_ref()
        .and_then(|r| r["content"].as_str())
        .unwrap_or_default();
    let total = content.lines().count();
    println!(
        "        {} 共 {total} 行;第 {offset} 行起至多 {limit} 行:",
        resolved["displayPath"].as_str().unwrap_or(path),
    );
    for (index, line) in content.lines().enumerate().skip(offset - 1).take(limit) {
        let text: String = line.chars().take(120).collect();
        println!("          {:>5}  {}", index + 1, text);
    }
}

/// 交互式外壳:用户敲命令,内核即时回答。
fn shell(mut kernel: Kernel, root: PathBuf) -> Result<(), String> {
    println!("== EvoBuddy 内核外壳(原生)==");
    println!("挂载根: {}(检索与 fs 都被限制在内)", root.display());
    println!("插件: {}", states_line(&kernel.states()));
    println!();
    println!("命令: find <模式...> | read <路径> [起始行] [行数] | use <模式...>");
    println!("      state | reports | stat <路径> | bench | root | help | quit");
    println!();

    let stdin = std::io::stdin();
    let mut id: u64 = 100;
    loop {
        print!("evobuddy> ");
        std::io::stdout().flush().ok();
        let mut line = String::new();
        if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
            println!();
            break; // Ctrl-D
        }
        let words: Vec<&str> = line.split_whitespace().collect();
        let (command, arguments) = match words.split_first() {
            Some(split) => split,
            None => continue,
        };
        match *command {
            "quit" | "q" | "exit" => break,
            "help" | "h" => {
                println!("  find <模式...>              一次多模式检索(aho-corasick 单遍)");
                println!("  read <路径> [起始行] [行数]  行号窗口读文件");
                println!("  use <模式...>               热插拔:替换 provider,consumer 重扫");
                println!("  state                      插件 fiber(id/代数/状态)");
                println!("  reports                    各代扫描历史");
                println!("  stat <路径>                目标元数据");
                println!("  bench                      全树吞吐(不截断命中)");
                println!("  root / quit");
            }
            "root" => println!("  {}", root.display()),
            "state" => {
                for state in kernel.states() {
                    println!(
                        "  {}@{} 第{}代 {}{}",
                        state.name,
                        state.id,
                        state.generation,
                        state.state,
                        state.error.as_ref().map(|e| format!(" ({e})")).unwrap_or_default(),
                    );
                }
            }
            "reports" => {
                let reports = kernel.reports();
                if reports.is_empty() {
                    println!("  (尚无扫描)");
                }
                for (generation, report) in reports.iter().enumerate() {
                    println!(
                        "  第{}代 {:?} -> {} 处 / {} 文件 / {} ms",
                        generation + 1,
                        report.patterns,
                        report.total_matches,
                        report.files_scanned,
                        round_ms(report.elapsed_ms),
                    );
                }
            }
            "find" if !arguments.is_empty() => {
                let patterns: Vec<String> = arguments.iter().map(|s| strip_quotes(s)).collect();
                id += 1;
                // 换模式 + 检索 = 一次热插拔加一次查询;闸门在链上先看根。
                kernel.handle(request(id, "plugins.reload", serde_json::json!({ "patterns": patterns })));
                id += 1;
                let outcome = show("find", kernel.handle(request(id, "search.find", serde_json::json!({}))));
                print_outcome(&outcome, 15);
            }
            "use" if !arguments.is_empty() => {
                let patterns: Vec<String> = arguments.iter().map(|s| strip_quotes(s)).collect();
                let before = kernel.states();
                id += 1;
                show("use", kernel.handle(request(id, "plugins.reload", serde_json::json!({ "patterns": patterns }))));
                println!("        fibers: {} -> {}", states_line(&before), states_line(&kernel.states()));
                let reports = kernel.reports();
                if let Some(latest) = reports.last() {
                    println!(
                        "        第{}代扫描: {:?} -> {} 处 / {} 文件 / {} ms",
                        reports.len(),
                        latest.patterns,
                        latest.total_matches,
                        latest.files_scanned,
                        round_ms(latest.elapsed_ms),
                    );
                }
            }
            "read" if !arguments.is_empty() => {
                let offset: usize = arguments.get(1).and_then(|v| v.parse().ok()).unwrap_or(1);
                let limit: usize = arguments.get(2).and_then(|v| v.parse().ok()).unwrap_or(20);
                print_read_window(&mut kernel, &mut id, arguments[0], offset.max(1), limit.max(1));
            }
            "stat" if !arguments.is_empty() => {
                id += 1;
                let resolved = kernel.handle(request(id, "fs.resolve", serde_json::json!({ "path": arguments[0] })));
                if !resolved.ok {
                    show("stat", resolved);
                    continue;
                }
                id += 1;
                let info = show(
                    "stat",
                    kernel.handle(request(id, "fs.stat", serde_json::json!({ "targetKey": resolved.result.unwrap()["targetKey"] }))),
                );
                println!("        {info}");
            }
            "bench" => {
                // 大上限的独立内核:命中不截断,bytes_scanned 才是真实遍历量。
                let mut limits = SearchLimits::default();
                limits.max_matches = BENCH_MAX_MATCHES;
                let patterns: Vec<String> = if arguments.is_empty() {
                    vec!["fn ".to_string(), "pub ".to_string(), "impl ".to_string()]
                } else {
                    arguments.iter().map(|s| strip_quotes(s)).collect()
                };
                let mut bench_kernel = Kernel::boot(root.clone(), patterns.clone(), limits)?;
                id += 1;
                let outcome = show("bench", bench_kernel.handle(request(id, "search.find", serde_json::json!({}))));
                let bytes = outcome["bytesScanned"].as_u64().unwrap_or(0);
                let ms = outcome["elapsedMs"].as_f64().unwrap_or(1.0);
                println!(
                    "        {} 个文件 / {} MB / {} ms = {:.0} MB/s(模式 {:?},{} 处)",
                    outcome["filesScanned"],
                    bytes / (1024 * 1024),
                    round_ms(ms),
                    (bytes as f64 / (1024.0 * 1024.0)) / (ms / 1000.0),
                    patterns,
                    outcome["totalMatches"],
                );
                bench_kernel.shutdown();
            }
            "find" | "use" | "read" | "stat" => println!("  需要参数;`help` 看用法"),
            other => println!("  没有命令 {other:?};`help` 看列表"),
        }
    }

    kernel.shutdown();
    println!("内核已关闭");
    Ok(())
}

/// 全树单遍多模式基准:命中不截断,遍历完整棵挂载树。
fn bench(mut kernel: Kernel) -> Result<(), String> {
    println!("== EvoBuddy 检索基准(单遍多模式,纯检索耗时)==");
    let outcome = show("search.find", kernel.handle(request(1, "search.find", serde_json::json!({}))));
    let bytes = outcome["bytesScanned"].as_u64().unwrap_or(0);
    let ms = outcome["elapsedMs"].as_f64().unwrap_or(1.0);
    println!(
        "  {} 个文件 / {} MB / {} 处命中 / {} ms = {:.0} MB/s",
        outcome["filesScanned"],
        bytes / (1024 * 1024),
        outcome["totalMatches"],
        round_ms(ms),
        (bytes as f64 / (1024.0 * 1024.0)) / (ms / 1000.0),
    );
    println!("  模式 {:?};命中上限不生效,遍历的是完整挂载树", strings(&outcome, "patterns"));
    kernel.shutdown();
    Ok(())
}

fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let first = args.next();
    let (mode, rest_root, patterns) = match first.as_deref() {
        Some("tour") | Some("shell") | Some("bench") => {
            let mode = first.unwrap().to_string();
            let rest_root = args.next().map(PathBuf::from);
            let rest: Vec<String> = args.collect();
            (mode, rest_root, rest)
        }
        // 兼容旧调用:cargo run -- [root] [patterns...] 跑 tour。
        other => (
            "tour".to_string(),
            other.map(PathBuf::from),
            args.collect(),
        ),
    };

    let root = rest_root
        .map(|r| r.canonicalize().unwrap_or(r))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    let patterns: Vec<String> = if patterns.is_empty() {
        vec!["fn ".to_string(), "pub ".to_string()]
    } else {
        patterns
    };

    match mode.as_str() {
        "shell" => shell(Kernel::boot(root.clone(), patterns, SearchLimits::default())?, root),
        "bench" => {
            let mut limits = SearchLimits::default();
            limits.max_matches = BENCH_MAX_MATCHES;
            bench(Kernel::boot(root, patterns, limits)?)
        }
        _ => {
            let kernel = Kernel::boot(root.clone(), patterns.clone(), SearchLimits::default())?;
            tour(kernel, &root)
        }
    }
}
