//! A GTK port of DSH's `@deepseek-ai/dsh-client-ui-message-feedback`.
//!
//! DSH client plugins are Cordis/React ES modules; they cannot be mounted by a
//! Rust GTK process. This crate ports the plugin's *declaration* — the
//! Like/Dislike pair in the assistant action strip — onto EvoBuddy's own
//! `MessageAction` contribution surface, so the effect can be evaluated in the
//! running app. It is deliberately labelled as a port, not a JS loader.

use eb_api::contrib::{self, MessageAction};
use eb_api::{EbPlugin, Ui};

pub struct DshFeedbackPlugin;

const ACTIONS: [MessageAction; 2] = [
    MessageAction {
        id: "dsh.feedback.like",
        icon: "thumb-up",
        tooltip: "Like this response (DSH feedback port)",
    },
    MessageAction {
        id: "dsh.feedback.dislike",
        icon: "thumb-down",
        tooltip: "Dislike this response (DSH feedback port)",
    },
];

impl EbPlugin for DshFeedbackPlugin {
    fn id(&self) -> &'static str {
        "dsh-feedback"
    }

    fn mount(&self, ui: &Ui) -> Result<(), eb_api::CordisError> {
        for action in ACTIONS {
            contrib::message_action(ui, action);
        }
        Ok(())
    }
}
