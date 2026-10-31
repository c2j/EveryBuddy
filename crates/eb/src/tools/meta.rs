//! Agent self-management tools: the turn plan the transcript renders as the
//! "Plan" card (WorkBuddy parity).

use super::*;

/// `todo_write(todos[])` — replace the turn's plan. Statuses use the same
/// words the renderer already knows: pending / in_progress / completed.
pub struct TodoWrite;

impl BuiltinTool for TodoWrite {
    fn name(&self) -> &'static str {
        "todo_write"
    }

    fn description(&self) -> &'static str {
        "Set or replace the plan for this turn: an ordered list of steps with \
statuses pending / in_progress / completed. Call it when a task needs more \
than two steps, and keep it current as you work — the user sees the plan as \
a live card."
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "todos": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "content": { "type": "string", "description": "One imperative step" },
                        "status": { "type": "string", "description": "pending | in_progress | completed" },
                    },
                    "required": ["content", "status"],
                },
                "description": "The full plan, in order"
            }
        })
    }

    fn run(&self, args: &serde_json::Value, ctx: &mut ToolContext, _ask: AskFn) -> String {
        let Some(todos) = args["todos"].as_array() else {
            return report(
                ctx,
                "Failed",
                "Plan".to_string(),
                "error: todos must be an array".into(),
            );
        };
        let entries: Vec<eb_api::model::PlanEntry> = todos
            .iter()
            .filter_map(|todo| {
                let content = todo["content"].as_str()?.trim().to_string();
                if content.is_empty() {
                    return None;
                }
                let status = match todo["status"].as_str() {
                    Some("in_progress") => "in_progress",
                    Some("completed") => "completed",
                    _ => "pending",
                };
                Some(eb_api::model::PlanEntry { content, status: status.to_string() })
            })
            .collect();
        if entries.is_empty() {
            return report(
                ctx,
                "Failed",
                "Plan".to_string(),
                "error: todos has no usable entries (need content + status)".into(),
            );
        }
        let count = entries.len();
        let done = entries
            .iter()
            .filter(|entry| entry.status == "completed")
            .count();
        let _ = ctx.sink.send(eb_api::AgentEvent::Block {
            block: eb_api::model::Block::Plan { entries },
        });
        // The transcript gets the Plan card above; this text goes back to the
        // model as the tool result.
        format!("plan updated: {done}/{count} completed")
    }
}
