mod events;
mod server;

pub use events::{IpcAction, IpcEvent};
pub use server::{init, send, set_action_handler, start_server, IPC_ADDR, IPC_PORT};