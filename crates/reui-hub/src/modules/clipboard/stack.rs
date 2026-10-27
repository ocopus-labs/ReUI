//! The paste stack: items pasting one per Ctrl+V, the next going on the clipboard each
//! time the user pastes, until they run out, Esc, or a minute idle.

use tokio::time::Instant;

use super::os::{Command, Content};
use super::view::Stack;
use super::{App, STACK_IDLE, STACK_KEY};

/// Items pasting one per Ctrl+V; `at` is the one on the clipboard now.
pub(super) struct PasteStack {
    pub(super) items: Vec<Content>,
    pub(super) at: usize,
    pub(super) idle_until: Instant,
}

impl App {
    pub(super) async fn start_stack(&mut self, ids: Vec<i64>) {
        self.end_stack();
        let items = self
            .db
            .call(move |s| {
                let mut items = Vec::new();
                for id in ids {
                    items.extend(s.content(id)?);
                }
                Ok(items)
            })
            .await
            .unwrap_or_default();
        let Some(first) = items.first().cloned() else { return };
        self.os.send(Command::Copy(first));
        self.os.watch_pastes(true);
        self.stack = Some(PasteStack { items, at: 0, idle_until: Instant::now() + STACK_IDLE });
        self.show_stack();
        self.send_state().await;
    }

    /// The user pasted the current item: the next one goes on the clipboard.
    pub(super) async fn advance_stack(&mut self) {
        let Some(stack) = self.stack.as_mut() else { return };
        stack.at += 1;
        stack.idle_until = Instant::now() + STACK_IDLE;
        match stack.items.get(stack.at).cloned() {
            Some(next) => {
                self.os.send(Command::Copy(next));
                self.show_stack();
            }
            None => self.end_stack(),
        }
        self.send_state().await;
    }

    pub(super) fn end_stack(&mut self) {
        if self.stack.take().is_some() {
            self.os.watch_pastes(false);
            self.ctx.retract(STACK_KEY);
        }
    }

    pub(super) fn stack_view(&self) -> Stack {
        match &self.stack {
            Some(s) => Stack { active: true, remaining: (s.items.len() - s.at.min(s.items.len())) as u32 },
            None => Stack::default(),
        }
    }
}
