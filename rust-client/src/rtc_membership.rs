//! Bounded MatrixRTC session membership for the native interoperability probe.
//! The caller supplies gomuks's RPC adapter. No keys or tokens belong in room state.
use serde_json::{Value, json};
use std::ops::AsyncFnMut;

pub const EVENT_TYPE: &str = "org.matrix.msc3401.call.member";

pub struct Membership {
    room: String,
    state_key: String,
    delay_id: String,
}

impl Membership {
    /// Advertise the room-wide voice slot for normal (numbered) room versions.
    /// The absolute expiry bounds stale membership even if the client dies between
    /// publishing the join and arming the homeserver's much shorter delayed leave.
    pub async fn join(
        room: &str,
        user: &str,
        device: &str,
        livekit_service: &url::Url,
        delay_ms: u32,
        mut rpc: impl AsyncFnMut(&'static str, Value) -> Result<Value, String>,
    ) -> Result<Self, String> {
        if device.is_empty() || !(1..60_000).contains(&delay_ms) {
            return Err("Invalid device or delayed-leave interval".into());
        }
        let mut this = Self {
            room: room.into(),
            state_key: format!("_{user}_{device}_m.call"),
            delay_id: String::new(),
        };
        let join = json!({
            "application":"m.call", "scope":"m.room", "call_id":"", "m.call.intent":"m.audio",
            "device_id":device, "membershipID":format!("{user}:{device}"), "expires":60_000,
            "focus_active":{"type":"livekit","focus_selection":"multi_sfu"},
            "foci_preferred":[{"type":"livekit","livekit_service_url":livekit_service.as_str()}]
        });
        rpc("set_state", this.state(join)).await?;
        let mut delayed = this.state(json!({}));
        delayed["delay_ms"] = delay_ms.into();
        let result = rpc("set_state", delayed).await;
        match result.as_ref().ok().and_then(Value::as_str) {
            Some(id) if !id.is_empty() && !id.starts_with('$') => this.delay_id = id.into(),
            _ => {
                rpc("set_state", this.state(json!({}))).await?;
                return Err("Could not arm delayed call leave".into());
            }
        }
        Ok(this)
    }

    fn state(&self, content: Value) -> Value {
        json!({"room_id":self.room,"type":EVENT_TYPE,"state_key":self.state_key,"content":content})
    }

    pub async fn heartbeat(
        &self,
        mut rpc: impl AsyncFnMut(&'static str, Value) -> Result<Value, String>,
    ) -> Result<(), String> {
        rpc(
            "update_delayed_event",
            json!({"delay_id": self.delay_id, "action": "restart"}),
        )
        .await?;
        Ok(())
    }

    pub async fn leave(
        self,
        mut rpc: impl AsyncFnMut(&'static str, Value) -> Result<Value, String>,
    ) -> Result<(), String> {
        if rpc(
            "update_delayed_event",
            json!({"delay_id":self.delay_id,"action":"send"}),
        )
        .await
        .is_err()
        {
            rpc("set_state", self.state(json!({}))).await?;
        }
        Ok(())
    }
}
