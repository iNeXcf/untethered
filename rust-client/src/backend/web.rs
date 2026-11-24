use super::{Commands, Events};
use futures_util::{FutureExt, select};
use serde_json::{Value, json};
use wasm_bindgen::{JsCast, closure::Closure};
use web_sys::{ErrorEvent, MessageEvent, Worker, WorkerOptions, WorkerType};

pub async fn run(commands: Commands, events: Events) -> Result<(), String> {
    let options = WorkerOptions::new();
    options.set_type(WorkerType::Module);
    let worker = Worker::new_with_options("./backend/gomuks-worker.js", &options)
        .map_err(|_| "Cannot start the embedded gomuks worker.")?;
    let (in_tx, incoming) = async_channel::unbounded();
    let errors = in_tx.clone();
    let receive = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
        let result = js_sys::JSON::stringify(&event.data())
            .ok()
            .and_then(|v| v.as_string())
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .ok_or("Invalid worker frame.");
        let _ = in_tx.try_send(result);
    });
    let error = Closure::<dyn FnMut(ErrorEvent)>::new(move |_| {
        let _ = errors.try_send(Err("gomuks worker failed to load."));
    });
    worker.set_onmessage(Some(receive.as_ref().unchecked_ref()));
    worker.set_onerror(Some(error.as_ref().unchecked_ref()));
    let result = async {
        loop {
            select! {
                message = incoming.recv().fuse() => {
                    let mut frame = message.map_err(|_| "Worker disconnected.")??;
                    if let Some(data) = frame["data"].as_str() {
                        frame["data"] = serde_json::from_str(data).map_err(|_| "Invalid worker data.")?;
                    }
                    if frame["command"] == "wasm-connection" && frame["data"]["connected"] == false {
                        return Err("gomuks worker stopped. Check browser storage permissions and reload.".into());
                    }
                    if events.send(frame).await.is_err() { return Ok(()); }
                }
                command = commands.recv().fuse() => {
                    let Ok(command) = command else { return Ok(()) };
                    let wire = json!({"command": command["command"], "request_id": command["request_id"], "data": command["data"].to_string()});
                    let message = js_sys::JSON::parse(&wire.to_string()).map_err(|_| "Invalid command.")?;
                    worker.post_message(&message).map_err(|_| "Cannot send to worker.")?;
                }
            }
        }
    }.await;
    worker.terminate();
    result
}
