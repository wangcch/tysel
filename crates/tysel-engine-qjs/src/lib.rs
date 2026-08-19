//! QuickJS-ng compatibility engine.
//!
//! Spike A (`roadmap.md` §26) pins a runtime to one worker thread, runs native
//! I/O on a Tokio reactor, and posts only bounded values through a completion
//! queue. JavaScript objects never leave the worker.

mod host;
mod isolate;
mod queue;

pub use isolate::{IsolateCancel, eval, eval_cancellable};

#[cfg(test)]
mod tests;
