//! The in-flight agent turn.
//!
//! An [`AnswerSource`] runs off the GTK main thread and reports progress through
//! an [`AgentSink`]. This module owns the receiving side: it drains events on
//! the main loop, applies them to [`crate::model::App`], and asks for a repaint.
//! Plugin code never sees a widget.
//!
//! The demo agent emits its whole turn synchronously inside `start`, so the
//! first drain usually completes before this function returns. The fake agent
//! (`EB_FAKE_AGENT=1`) sleeps between chunks, which is what exercises the timer
//! and cancellation paths.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use eb_api::{AgentEvent, AgentSink, AgentStream, AgentStreamError, AnswerHandle, AnswerSource};

use crate::model::Store;
use crate::shell::Refresher;

struct Active {
    task: usize,
    handle: Box<dyn AnswerHandle>,
    stream: AgentStream,
}

pub struct Runner {
    store: Store,
    refresher: Refresher,
    source: Arc<dyn AnswerSource>,
    active: RefCell<Option<Active>>,
    /// One timer serves whichever answer is current; starting another timer per
    /// submit would double-drain the same stream.
    polling: Cell<bool>,
}

impl Runner {
    pub fn new(store: Store, refresher: Refresher, source: Arc<dyn AnswerSource>) -> Rc<Self> {
        Rc::new(Runner {
            store,
            refresher,
            source,
            active: RefCell::new(None),
            polling: Cell::new(false),
        })
    }

    /// Start an answer for `text`. A previous in-flight answer is marked
    /// cancelled before the new task is created, so a task can never be left
    /// "streaming" after the user moves on.
    pub fn submit(self: &Rc<Self>, text: String) {
        self.supersede();

        let task = self.store.borrow_mut().begin_task(&text);
        let Some(task) = task else {
            self.refresher.request();
            return;
        };

        let (sink, stream) = AgentSink::channel();
        let handle = self.source.start(&text, sink);
        *self.active.borrow_mut() = Some(Active {
            task,
            handle,
            stream,
        });

        // Synchronous sources (the demo agent) have already sent everything.
        self.drain();
        self.schedule();
        self.refresher.request();

        if let Some(ms) = cancel_after_ms() {
            let runner = self.clone();
            gtk::glib::timeout_add_local_once(Duration::from_millis(ms), move || {
                runner.cancel();
            });
        }
    }

    /// Cancel the in-flight answer; the producer reports the terminal event and
    /// the normal drain path marks the turn finished.
    pub fn cancel(&self) {
        if std::env::var_os("EB_DEBUG").is_some() {
            eprintln!("agent: cancel requested");
        }
        if let Some(active) = self.active.borrow().as_ref() {
            active.handle.cancel();
        }
    }

    /// Whether an answer is still streaming. The fake-agent harness polls this
    /// and quits when the stream reaches a terminal event.
    pub fn is_busy(&self) -> bool {
        self.active.borrow().is_some()
    }

    fn supersede(&self) {
        let task = self.active.borrow().as_ref().map(|active| active.task);
        if let Some(task) = task {
            self.store.borrow_mut().apply_agent_event(
                task,
                AgentEvent::Failed {
                    message: "Cancelled".to_string(),
                },
            );
        }
        if let Some(active) = self.active.borrow_mut().take() {
            active.handle.cancel();
        }
    }

    fn drain(self: &Rc<Self>) {
        let mut closed = false;
        {
            let mut slot = self.active.borrow_mut();
            let Some(active) = slot.as_mut() else {
                return;
            };
            loop {
                match active.stream.try_recv() {
                    Ok(event) => {
                        if std::env::var_os("EB_DEBUG").is_some() {
                            eprintln!("agent: task {} event {event:?}", active.task);
                        }
                        let done =
                            matches!(event, AgentEvent::Finished | AgentEvent::Failed { .. });
                        self.store
                            .borrow_mut()
                            .apply_agent_event(active.task, event);
                        if done {
                            closed = true;
                            break;
                        }
                    }
                    Err(AgentStreamError::Empty | AgentStreamError::Timeout) => break,
                    Err(AgentStreamError::Closed) => {
                        self.store.borrow_mut().apply_agent_event(
                            active.task,
                            AgentEvent::Failed {
                                message: "agent stream closed".to_string(),
                            },
                        );
                        closed = true;
                        break;
                    }
                }
            }
        }
        if closed {
            self.active.borrow_mut().take();
        }
    }

    fn schedule(self: &Rc<Self>) {
        if self.active.borrow().is_none() || self.polling.replace(true) {
            return;
        }
        let runner = self.clone();
        gtk::glib::timeout_add_local(Duration::from_millis(16), move || {
            runner.drain();
            runner.refresher.request();
            if runner.active.borrow().is_some() {
                gtk::glib::ControlFlow::Continue
            } else {
                runner.polling.set(false);
                gtk::glib::ControlFlow::Break
            }
        });
    }
}

impl Drop for Runner {
    fn drop(&mut self) {
        if let Some(active) = self.active.get_mut().take() {
            active.handle.cancel();
        }
    }
}

fn cancel_after_ms() -> Option<u64> {
    std::env::var("EB_CANCEL_AFTER_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
}
