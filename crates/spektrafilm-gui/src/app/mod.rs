//! Application shell: state ownership, render dispatch and export coordination.
mod export;
mod history;
mod message;
mod render;
mod state;

pub(crate) use message::Message;
pub(crate) use state::App;
