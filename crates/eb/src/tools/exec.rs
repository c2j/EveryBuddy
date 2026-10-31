//! Command execution: synchronous `run_command`, plus a background trio —
//! `run_command(run_in_background)`, `command_output`, `kill_command` — so a
//! long build or test run does not block a whole tool round. Background jobs
//! live in a turn-scoped [`JobTable`]: the streaming loop keeps it alive
//! across rounds, which is what makes "start, keep working, poll later"
//! possible for the model.

use std::collections::BTreeMap;
use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use super::*;

/// Output kept per background job (tail-trimmed on read).
const JOB_OUTPUT_CAP: usize = 64_000;

/// One running background command: the child, a reader thread feeding a
/// shared buffer, and a done flag.
pub struct Job {
    child: Arc<Mutex<Option<Child>>>,
    buffer: Arc<Mutex<String>>,
    done: Arc<AtomicBool>,
    /// Bytes dropped from the head of the buffer to respect the cap.
    dropped: Arc<AtomicU32>,
}

impl Job {
    fn spawn(command: &str, root: &std::path::Path) -> Result<Job, String> {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("spawn failed: {error}"))?;
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let child = Arc::new(Mutex::new(Some(child)));
        let buffer: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
        let done = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU32::new(0));

        let reader_child = child.clone();
        let reader_buffer = buffer.clone();
        let reader_done = done.clone();
        let reader_dropped = dropped.clone();
        std::thread::Builder::new()
            .name("evobuddy-job-output".into())
            .spawn(move || {
                let mut sources: Vec<Box<dyn Read + Send>> = Vec::new();
                if let Some(stdout) = stdout {
                    sources.push(Box::new(stdout));
                }
                if let Some(stderr) = stderr {
                    sources.push(Box::new(stderr));
                }
                let mut scratch = [0u8; 4_096];
                loop {
                    let mut progressed = false;
                    for source in sources.as_mut_slice() {
                        loop {
                            match source.read(&mut scratch) {
                                Ok(0) => break,
                                Ok(n) => {
                                    progressed = true;
                                    if let Ok(mut buffer) = reader_buffer.lock() {
                                        buffer.push_str(&String::from_utf8_lossy(&scratch[..n]));
                                        let excess = buffer.len().saturating_sub(JOB_OUTPUT_CAP);
                                        if excess > 0 {
                                            // Trim at a char boundary.
                                            let cut = {
                                                let text = buffer.as_str();
                                                let mut cut = excess;
                                                while cut < text.len()
                                                    && !text.is_char_boundary(cut)
                                                {
                                                    cut += 1;
                                                }
                                                cut
                                            };
                                            buffer.drain(..cut);
                                            reader_dropped.fetch_add(
                                                cut as u32,
                                                Ordering::SeqCst,
                                            );
                                        }
                                    }
                                }
                                Err(_) => break,
                            }
                        }
                    }
                    if !progressed {
                        // Both pipes are at EOF; check whether the child has
                        // exited, then finish.
                        let exited = match reader_child.lock() {
                            Ok(mut slot) => match slot.as_mut() {
                                Some(child) => child
                                    .try_wait()
                                    .map(|status| status.is_some())
                                    .unwrap_or(false),
                                None => true,
                            },
                            Err(_) => true,
                        };
                        if exited {
                            reader_done.store(true, Ordering::SeqCst);
                            // Reap so the entry does not linger as a zombie.
                            if let Ok(mut slot) = reader_child.lock() {
                                if let Some(child) = slot.as_mut() {
                                    let _ = child.wait();
                                }
                            }
                            return;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
            })
            .ok();

        Ok(Job {
            child,
            buffer,
            done,
            dropped,
        })
    }

    fn snapshot(&self) -> (String, bool) {
        let text = self
            .buffer
            .lock()
            .map(|buffer| buffer.clone())
            .unwrap_or_default();
        (text, self.done.load(Ordering::SeqCst))
    }
}

/// Turn-scoped table of background jobs, keyed by the id reported to the
/// model. `run_command` inserts; `command_output`/`kill_command` consume.
#[derive(Default)]
pub struct JobTable {
    next_id: u32,
    jobs: BTreeMap<u32, Job>,
}

impl JobTable {
    fn insert(&mut self, job: Job) -> u32 {
        self.next_id += 1;
        let id = self.next_id;
        self.jobs.insert(id, job);
        id
    }

    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    /// Finished jobs whose output has been collected are reaped to keep the
    /// table small over a long turn.
    fn reap(&mut self) {
        self.jobs.retain(|_, job| !job.done.load(Ordering::SeqCst));
    }
}

/// `run_command(command, run_in_background?)` — run a shell command in the
/// workspace. Synchronous by default (exit code + output in one result);
/// with `run_in_background` it returns a job id immediately for later
/// `command_output` / `kill_command` calls.
pub struct RunCommand;

impl BuiltinTool for RunCommand {
    fn name(&self) -> &'static str {
        "run_command"
    }

    fn description(&self) -> &'static str {
        "Run a shell command in the task workspace and capture its output. \
Set run_in_background=true for long builds or servers: you get a job id, \
then poll with command_output and stop it with kill_command. Requires \
permission."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "command": { "type": "string", "description": "Shell command" },
            "run_in_background": { "type": "boolean", "description": "Start without waiting; returns a job id (default false)" }
        })
    }

    fn class(&self) -> PermClass {
        PermClass::Command
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let command = args["command"].as_str().unwrap_or_default().to_string();
        if args["run_in_background"].as_bool().unwrap_or(false) {
            match Job::spawn(&command, ctx.root) {
                Ok(job) => {
                    let id = ctx.jobs.insert(job);
                    report(
                        ctx,
                        "Ran",
                        format!("Bash(bg): {command}"),
                        format!("started background job {id}; poll with command_output(id={id})"),
                    )
                }
                Err(error) => report(
                    ctx,
                    "Failed",
                    format!("Bash(bg): {command}"),
                    error,
                ),
            }
        } else {
            let output = Command::new("sh")
                .arg("-c")
                .arg(&command)
                .current_dir(ctx.root)
                .stdin(Stdio::null())
                .output();
            match output {
                Ok(output) => {
                    let mut combined = String::new();
                    combined.push_str(&String::from_utf8_lossy(&output.stdout));
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !stderr.trim().is_empty() {
                        combined.push_str(&stderr);
                    }
                    let code = output.status.code().unwrap_or(-1);
                    let body = truncate_tail(combined.trim(), TOOL_OUTPUT_CAP);
                    report(
                        ctx,
                        if code == 0 { "Ran" } else { "Failed" },
                        format!("Bash: {command}"),
                        format!("exit {code}\n{body}"),
                    )
                }
                Err(error) => report(
                    ctx,
                    "Failed",
                    format!("Bash: {command}"),
                    format!("spawn failed: {error}"),
                ),
            }
        }
    }
}

fn job_args_error(args: &serde_json::Value, ctx: &mut ToolContext, label: &str) -> Option<u32> {
    match args["id"].as_u64() {
        Some(id) => Some(id as u32),
        None => {
            report(
                ctx,
                "Failed",
                label.to_string(),
                "error: id must be the number returned by run_command".into(),
            );
            None
        }
    }
}

/// `command_output(id)` — current output of one background job.
pub struct CommandOutput;

impl BuiltinTool for CommandOutput {
    fn name(&self) -> &'static str {
        "command_output"
    }

    fn description(&self) -> &'static str {
        "Read the output so far of a background command started with \
run_command(run_in_background=true), and whether it has finished."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "id": { "type": "integer", "description": "Job id returned by run_command" }
        })
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let Some(id) = job_args_error(args, ctx, "command_output") else {
            return "error: bad id".into();
        };
        let Some(job) = ctx.jobs.jobs.get(&id) else {
            return report(
                ctx,
                "Failed",
                format!("Output job {id}"),
                format!("no such job {id}; it may already be reaped"),
            );
        };
        let (text, done) = job.snapshot();
        let status = if done {
            "finished".to_string()
        } else {
            "still running".to_string()
        };
        let trimmed = if text.len() > TOOL_OUTPUT_CAP {
            format!("[… earlier output trimmed …]\n{}", &text[text.len() - TOOL_OUTPUT_CAP..])
        } else {
            text
        };
        report(
            ctx,
            "Ran",
            format!("Output job {id}"),
            format!("{status}\n{}", if trimmed.is_empty() { "(no output yet)" } else { &trimmed }),
        )
    }
}

/// `kill_command(id)` — stop one background command.
pub struct KillCommand;

impl BuiltinTool for KillCommand {
    fn name(&self) -> &'static str {
        "kill_command"
    }

    fn description(&self) -> &'static str {
        "Stop a background command started with run_command(run_in_background=true)."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "id": { "type": "integer", "description": "Job id returned by run_command" }
        })
    }

    fn class(&self) -> PermClass {
        PermClass::Command
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let Some(id) = job_args_error(args, ctx, "kill_command") else {
            return "error: bad id".into();
        };
        let Some(job) = ctx.jobs.jobs.get(&id) else {
            return report(
                ctx,
                "Failed",
                format!("Kill job {id}"),
                format!("no such job {id}"),
            );
        };
        let killed = match job.child.lock() {
            Ok(mut slot) => match slot.as_mut() {
                Some(child) => child.kill().is_ok(),
                None => false,
            },
            Err(_) => false,
        };
        report(
            ctx,
            "Ran",
            format!("Kill job {id}"),
            if killed {
                format!("job {id} killed")
            } else {
                format!("job {id} already exited")
            },
        )
    }
}
