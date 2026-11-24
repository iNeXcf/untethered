use async_channel::{Receiver, Sender};
use serde_json::Value;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::run;
#[cfg(target_arch = "wasm32")]
mod web;
#[cfg(target_arch = "wasm32")]
pub use web::run;

type Commands = Receiver<Value>;
type Events = Sender<Value>;
