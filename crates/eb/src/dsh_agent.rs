//! Answer source backed by the rutis dsh agent, so the session and the UI turn
//! come from the same agent loop.

use std::sync::Arc;

use eb_api::{AgentEvent, AgentSink, AnswerHandle, AnswerSource, EbPlugin, NoopHandle, Ui};

pub struct DshAgent;

static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn enabled() -> bool {
    ACTIVE.load(std::sync::atomic::Ordering::SeqCst)
}

pub fn mark_enabled() {
    ACTIVE.store(true, std::sync::atomic::Ordering::SeqCst);
}

impl EbPlugin for DshAgent {
    fn id(&self) -> &'static str {
        "dsh-agent"
    }

    fn mount(&self, ui: &Ui) -> Result<(), eb_api::CordisError> {
        eb_api::provide_answer(ui.ctx, Arc::new(DshAgent)).map(|_| ())
    }
}

impl AnswerSource for DshAgent {
    fn start(&self, prompt: &str, sink: AgentSink) -> Box<dyn AnswerHandle> {
        // `start` must return promptly: `run_agent` is a blocking HTTP call
        // that runs a real agent turn (minutes, not milliseconds). Doing it
        // inline would park the runtime thread and freeze the main thread in
        // the start handshake.
        let prompt = prompt.to_string();
        std::thread::Builder::new()
            .name("evobuddy-dsh-agent".into())
            .spawn(move || {
                if sink
                    .send(AgentEvent::Started { name: "dsh".into() })
                    .is_err()
                {
                    return;
                }
                match crate::bridge::run_agent(&prompt) {
                    Ok(messages) => {
                        for (role, text) in messages {
                            if role == "assistant" && !text.is_empty() {
                                if sink
                                    .send(AgentEvent::Block {
                                        block: eb_api::model::Block::Text(text),
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        }
                        let _ = sink.send(AgentEvent::Finished);
                    }
                    Err(error) => {
                        let _ = sink.send(AgentEvent::Failed { message: error });
                    }
                }
            })
            .expect("spawn dsh agent worker");
        Box::new(NoopHandle)
    }
}
