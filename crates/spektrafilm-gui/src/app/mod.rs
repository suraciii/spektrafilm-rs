//! Application shell: state ownership, render dispatch and export coordination.
pub(crate) mod comparison;
mod export;
pub(crate) mod export_queue;
pub(crate) mod history;
mod message;
mod render;
mod state;

pub(crate) use message::Message;
pub(crate) use state::App;
