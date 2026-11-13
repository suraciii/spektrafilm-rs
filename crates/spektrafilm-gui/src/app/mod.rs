//! Application shell: state ownership, input/material transitions, render
//! dispatch and export coordination.
mod dialog;
mod export;
pub(crate) mod export_queue;
pub(crate) mod history;
mod input;
mod lut;
mod material;
mod message;
mod render;
mod state;

pub(crate) use message::Message;
pub(crate) use state::App;
