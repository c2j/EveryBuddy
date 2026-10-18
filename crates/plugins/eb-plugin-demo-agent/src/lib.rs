//! The demo agent: the stand-in behind the composer.
//!
//! This build has no model behind it, so a submitted prompt gets fixed
//! placeholder turns that deliberately exercise every message block type the
//! real client draws (ui-spec §9: nothing pretends a model replied — the copy
//! says so). It implements [`AnswerSource`], so swapping in a real agent later
//! is a matter of providing that service from somewhere else; the host and the
//! renderer never change.
//!
//! `EB_FAKE_AGENT=1` swaps in a deliberately slow streaming agent. It exists to
//! exercise the event contract and the GTK-side drain loop without waiting for
//! a model: `EB_FAKE_AGENT_DELAY_MS` (default 120) and `EB_FAKE_AGENT_CHUNKS`
//! (default 8) control the stream.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use eb_api::model::{Artifact, Block, DiffLine};
use eb_api::{
    AgentEvent, AgentSink, AnswerHandle, AnswerSource, EbPlugin, NoopHandle, Ui,
};

pub struct DemoAgent {
    /// Which placeholder answer comes next, so repeated sends look different.
    answers: AtomicUsize,
}

impl Clone for DemoAgent {
    fn clone(&self) -> Self {
        // The counter's *value* carries over: a cloned agent continues the
        // alternation rather than restarting it.
        DemoAgent {
            answers: AtomicUsize::new(self.answers.load(Ordering::SeqCst)),
        }
    }
}

impl DemoAgent {
    pub fn new() -> Self {
        DemoAgent {
            answers: AtomicUsize::new(0),
        }
    }
}

impl Default for DemoAgent {
    fn default() -> Self {
        Self::new()
    }
}

impl EbPlugin for DemoAgent {
    fn id(&self) -> &'static str {
        "demo-agent"
    }

    fn mount(&self, ui: &Ui) -> Result<(), eb_api::CordisError> {
        // Provided as a rutis service, not a contribution: the composer's send
        // path resolves it the same way a real agent would be resolved.
        let source: Arc<dyn AnswerSource> = if std::env::var_os("EB_FAKE_AGENT").is_some() {
            Arc::new(FakeAgent::from_env())
        } else {
            Arc::new(self.clone())
        };
        eb_api::provide_answer(ui.ctx, source).map(|_| ())
    }
}

impl AnswerSource for DemoAgent {
    fn start(&self, _prompt: &str, sink: AgentSink) -> Box<dyn AnswerHandle> {
        let nth = self.answers.fetch_add(1, Ordering::SeqCst);
        let (blocks, artifact) = placeholder_answer(nth);

        if sink
            .send(AgentEvent::Started {
                name: "EvoBuddy".to_string(),
            })
            .is_err()
        {
            return Box::new(NoopHandle);
        }
        for block in blocks {
            if sink.send(AgentEvent::Block { block }).is_err() {
                return Box::new(NoopHandle);
            }
        }
        if let Some(artifact) = artifact {
            let _ = sink.send(AgentEvent::Artifact { artifact });
        }
        let _ = sink.send(AgentEvent::Finished);
        Box::new(NoopHandle)
    }
}

/// A slow, cancellable stream used by `EB_FAKE_AGENT=1`. The worker is a plain
/// thread on purpose: it makes the test independent of the host runtime that
/// M2.0b will add.
pub struct FakeAgent {
    delay: Duration,
    chunks: usize,
}

impl FakeAgent {
    pub fn new(delay: Duration, chunks: usize) -> Self {
        FakeAgent { delay, chunks }
    }

    pub fn from_env() -> Self {
        let delay_ms = std::env::var("EB_FAKE_AGENT_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(120);
        let chunks = std::env::var("EB_FAKE_AGENT_CHUNKS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(8)
            .max(1);
        FakeAgent::new(Duration::from_millis(delay_ms), chunks)
    }
}

impl AnswerSource for FakeAgent {
    fn start(&self, _prompt: &str, sink: AgentSink) -> Box<dyn AnswerHandle> {
        // Test hook: prove the host turns a panicking `start` into a Failed
        // event instead of taking the window down.
        if std::env::var_os("EB_FAKE_PANIC_START").is_some() {
            panic!("fake agent: deliberate start panic");
        }
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let delay = self.delay;
        let chunks = self.chunks;

        std::thread::spawn(move || {
            if sink
                .send(AgentEvent::Started {
                    name: "Fake agent".to_string(),
                })
                .is_err()
            {
                return;
            }
            let _ = sink.send(AgentEvent::Status {
                text: "Streaming".to_string(),
            });

            for index in 0..chunks {
                if flag.load(Ordering::SeqCst) {
                    let _ = sink.send(AgentEvent::Failed {
                        message: "Cancelled".to_string(),
                    });
                    return;
                }
                std::thread::sleep(delay);
                if flag.load(Ordering::SeqCst) {
                    let _ = sink.send(AgentEvent::Failed {
                        message: "Cancelled".to_string(),
                    });
                    return;
                }
                if sink
                    .send(AgentEvent::Delta {
                        text: format!("chunk {index} "),
                    })
                    .is_err()
                {
                    return;
                }
            }

            if sink
                .send(AgentEvent::Block {
                    block: Block::Text("Fake stream finished.".to_string()),
                })
                .is_err()
            {
                return;
            }
            let _ = sink.send(AgentEvent::Usage {
                used: 42,
                total: 100,
            });
            let _ = sink.send(AgentEvent::Finished);
        });

        Box::new(FlagHandle { cancelled })
    }
}

struct FlagHandle {
    cancelled: Arc<AtomicBool>,
}

impl AnswerHandle for FlagHandle {
    fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
    }
}

/// The stand-in reply. Two variants, so repeated sends look different, and each
/// one deliberately covers text, tool and table blocks.
///
/// The status stays `None` and the turn finishes before the host paints: the
/// capture only ever showed a processing status *while* a turn was arriving,
/// which the demo agent never does. The event path still supports both.
fn placeholder_answer(nth: usize) -> (Vec<Block>, Option<Artifact>) {
    if nth % 2 == 1 {
        let output = (1..=20)
            .map(|n| n.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        return (
            vec![
                Block::Text(
                    "This second placeholder answer exercises the M2.1 content blocks: a \
                     diff, a command with a scrollable output, and a table."
                        .to_string(),
                ),
                Block::Diff {
                    path: "demo/main.rs".to_string(),
                    added: 7,
                    removed: 1,
                    lines: vec![
                        DiffLine::Context("fn main() {".to_string()),
                        DiffLine::Remove("    let old = 1;".to_string()),
                        DiffLine::Add("    // EvoBuddy placeholder diff.".to_string()),
                        DiffLine::Add("    let value = 42;".to_string()),
                        DiffLine::Context("    println!(\"{value}\");".to_string()),
                        DiffLine::Context("}".to_string()),
                    ],
                },
                Block::Exec {
                    command: "seq 1 20".to_string(),
                    output,
                    exit_code: Some(0),
                },
                Block::Table {
                    headers: vec!["Block".to_string(), "Renders as".to_string()],
                    rows: vec![
                        vec!["Diff".to_string(), "Light-green card".to_string()],
                        vec!["Exec".to_string(), "Scrollable command output".to_string()],
                        vec!["Table".to_string(), "Rounded card".to_string()],
                    ],
                },
            ],
            None,
        );
    }

    let artifact = Artifact {
        name: "demo/main.rs".to_string(),
        code: PLACEHOLDER_CODE.to_string(),
    };

    (
        vec![
            Block::Text(
                "Nothing is running behind this window yet — EvoBuddy is a UI build, so this \
                 reply is fixed text rather than a model's answer. What follows exercises the \
                 message vocabulary the real client draws."
                    .to_string(),
            ),
            Block::Tool {
                status: "Wrote".to_string(),
                primary: "demo/main.rs".to_string(),
                detail: Some("+42 −0".to_string()),
            },
            Block::Text(
                "A table renders as a rounded card with a tinted header row and hairline \
                 separators."
                    .to_string(),
            ),
            Block::Table {
                headers: vec!["Block".to_string(), "Renders as".to_string()],
                rows: vec![
                    vec!["Text".to_string(), "Paragraph on a 24px line".to_string()],
                    vec!["Tool".to_string(), "One muted 26px row".to_string()],
                    vec!["Table".to_string(), "A card with a tinted header".to_string()],
                ],
            },
            Block::Text(
                "Because this turn wrote a file, the artifact panel opens beside the \
                 conversation and the panes can be dragged."
                    .to_string(),
            ),
        ],
        Some(artifact),
    )
}

/// Stand-in artifact body. Deliberately self-describing so nobody mistakes it
/// for a real generated file.
const PLACEHOLDER_CODE: &str = r#"// Placeholder artifact for the EvoBuddy UI build.
//
// The real client shows a generated file here in a Monaco editor. This pane
// only needs to demonstrate the frame: a tab strip, a title row, and a
// monospaced body that scrolls.

fn main() {
    let message = "no agent is attached to this build";
    println!("{message}");
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use eb_api::AgentStream;

    fn drain(stream: &AgentStream) -> Vec<AgentEvent> {
        let mut events = Vec::new();
        loop {
            match stream.recv_timeout(Duration::from_millis(500)) {
                Ok(event) => {
                    let done = matches!(event, AgentEvent::Finished | AgentEvent::Failed { .. });
                    events.push(event);
                    if done {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        events
    }

    #[test]
    fn demo_emits_blocks_then_finishes() {
        let agent = DemoAgent::new();
        let (sink, stream) = AgentSink::channel();
        let _handle = agent.start("hello", sink);
        let events = drain(&stream);
        assert!(matches!(events.first(), Some(AgentEvent::Started { .. })));
        assert!(matches!(events.last(), Some(AgentEvent::Finished)));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::Block { block: Block::Table { .. } })));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::Artifact { .. })));
    }

    #[test]
    fn second_demo_answer_has_the_m21_blocks() {
        let agent = DemoAgent::new();
        let (sink, stream) = AgentSink::channel();
        let _first = agent.start("hello", sink);
        let _ = drain(&stream);
        let (sink, stream) = AgentSink::channel();
        let _second = agent.start("hello", sink);
        let events = drain(&stream);
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Block {
                block: Block::Diff { .. }
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Block {
                block: Block::Exec { .. }
            }
        )));
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::Block {
                block: Block::Table { .. }
            }
        )));
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::Artifact { .. })));
    }

    #[test]
    fn fake_agent_streams_and_finishes() {
        let agent = FakeAgent::new(Duration::from_millis(1), 3);
        let (sink, stream) = AgentSink::channel();
        let _handle = agent.start("hello", sink);
        let events = drain(&stream);
        assert!(matches!(events.last(), Some(AgentEvent::Finished)));
        let deltas = events
            .iter()
            .filter(|event| matches!(event, AgentEvent::Delta { .. }))
            .count();
        assert_eq!(deltas, 3);
    }

    #[test]
    fn fake_agent_stops_when_cancelled() {
        let agent = FakeAgent::new(Duration::from_millis(20), 50);
        let (sink, stream) = AgentSink::channel();
        let handle = agent.start("hello", sink);
        // Let the worker start, then cancel it.
        let first = stream.recv_timeout(Duration::from_millis(500)).unwrap();
        assert!(matches!(first, AgentEvent::Started { .. }));
        handle.cancel();
        let events = drain(&stream);
        assert!(matches!(
            events.last(),
            Some(AgentEvent::Failed { message }) if message == "Cancelled"
        ));
        assert!(!events
            .iter()
            .any(|event| matches!(event, AgentEvent::Finished)));
    }

    #[test]
    fn sending_to_a_dropped_stream_fails() {
        let (sink, stream) = AgentSink::channel();
        drop(stream);
        assert!(sink.send(AgentEvent::Finished).is_err());
    }
}
