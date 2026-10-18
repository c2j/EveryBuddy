//! The demo agent: the stand-in behind the composer.
//!
//! This build has no model behind it, so a submitted prompt gets fixed
//! placeholder turns that deliberately exercise every message block type the
//! real client draws (ui-spec §9: nothing pretends a model replied — the copy
//! says so). It implements [`AnswerSource`], so swapping in a real agent later
//! is a matter of providing that service from somewhere else; the host and the
//! renderer never change.

use std::sync::atomic::{AtomicUsize, Ordering};

use eb_api::model::{Artifact, Block, Turn};
use eb_api::{AnswerSource, EbPlugin, Ui};
use std::sync::Arc;

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
        DemoAgent { answers: AtomicUsize::new(0) }
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
        eb_api::provide_answer(ui.ctx, Arc::new(self.clone())).map(|_| ())
    }
}

impl AnswerSource for DemoAgent {
    fn answer(&self, _prompt: &str) -> (Turn, Option<Artifact>) {
        let nth = self.answers.fetch_add(1, Ordering::SeqCst);
        placeholder_answer(nth)
    }
}

/// The stand-in reply. Two variants, so repeated sends look different, and each
/// one deliberately covers text, tool and table blocks.
///
/// `status` is always `None` and `streaming` always `false`: the capture only
/// ever showed a processing status *while* a turn was arriving, which cannot
/// happen without an agent. The renderer still handles both fields so that
/// wiring a real backend later does not require touching it.
fn placeholder_answer(nth: usize) -> (Turn, Option<Artifact>) {
    let agent = |blocks: Vec<Block>| Turn::Agent {
        name: "EvoBuddy".to_string(),
        status: None,
        blocks,
        streaming: false,
    };

    if nth % 2 == 1 {
        return (
            agent(vec![
                Block::Text(
                    "Nothing is running behind this window yet — EvoBuddy is a UI build, so \
                     this reply is fixed text rather than a model's answer. The blocks below \
                     are here so every message type the real client draws has something to \
                     render."
                        .to_string(),
                ),
                Block::Tool {
                    status: "Ran".to_string(),
                    primary: "echo placeholder".to_string(),
                    detail: Some("0.02s".to_string()),
                },
                Block::Text(
                    "A tool row is one muted line: a status, the command, and how long it \
                     took."
                        .to_string(),
                ),
            ]),
            None,
        );
    }

    let artifact = Artifact {
        name: "demo/main.rs".to_string(),
        code: PLACEHOLDER_CODE.to_string(),
    };

    (
        agent(vec![
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
        ]),
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
