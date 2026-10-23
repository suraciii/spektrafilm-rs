//! Application shell: state ownership, render dispatch and export coordination.
mod export;
mod history;
mod render;
mod state;

pub(crate) use state::App;
