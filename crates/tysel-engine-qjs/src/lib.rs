//! QuickJS-ng compatibility engine.
//!
//! Spike A pins a runtime to one worker thread and settles promises through a
//! completion queue. Spike B reuses that isolate as a fetch-handler pool behind
//! a native HTTP listener.

mod fetch;
mod host;
mod isolate;
mod pool;
mod queue;

pub use isolate::{IsolateCancel, eval, eval_cancellable, eval_with_reactor};
pub use pool::IsolatePool;
pub use queue::{IoCompletion, IoRequest, OpId, Reactor, open_bridge};

#[cfg(test)]
mod tests;
