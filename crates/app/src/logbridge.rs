//! A bridge from `tracing` to the window log.
//!
//! The core writes its messages through `tracing` (placed there by `throne-ipc`),
//! and the same messages must be visible in the interface — otherwise diagnosing
//! a connection failure requires using the terminal.

use async_channel::Sender;
use tracing::field::{Field, Visit};
use tracing::{Event as TracingEvent, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use crate::engine::Event;

pub struct LogBridge {
    sender: Sender<Event>,
}

impl LogBridge {
    pub fn new(sender: Sender<Event>) -> Self {
        Self { sender }
    }
}

#[derive(Default)]
struct MessageVisitor {
    message: String,
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{value:?}").trim_matches('"').to_string();
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        }
    }
}

impl<S: Subscriber> Layer<S> for LogBridge {
    fn on_event(&self, event: &TracingEvent<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        if visitor.message.is_empty() {
            return;
        }

        let metadata = event.metadata();
        let line = if metadata.target() == "core" {
            visitor.message
        } else {
            format!("[{}] {}", metadata.target(), visitor.message)
        };

        // Do not block on the channel: the log is not worth slowing the core down.
        let _ = self.sender.try_send(Event::Log(line));
    }
}
