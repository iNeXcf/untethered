//! Linux one-to-one voice. Matrix state/Olm stay in the existing gomuks device;
//! only the media and audio streams live here, for the lifetime of a call.
use async_channel::{Receiver, Sender};
use futures_util::{SinkExt, StreamExt};
use livekit::{
    e2ee::{E2eeOptions, EncryptionType, key_provider::KeyProvider},
    prelude::*,
    webrtc::{
        audio_source::native::NativeAudioSource,
        audio_stream::native::NativeAudioStream,
        prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource},
    },
};
use serde_json::{Value, json};
use std::{
    net::SocketAddr,
    process::Stdio,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    process::Command,
    task::JoinSet,
    time::{self, Instant},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest},
};
use untethered_iced::rtc_keys::{self, EVENT as KEY, SenderKey};
use untethered_iced::rtc_membership::{EVENT_TYPE, Membership};
use url::Url;

type Result<T> = std::result::Result<T, &'static str>;
const SLOT: &str = "org.matrix.msc4143.rtc.slot";
const RING: &str = "org.matrix.msc4075.rtc.notification";
const SLOT_ID: &str = "m.call#ROOM";
const ENDED: &str = "Call ended.";
const SECOND: Duration = Duration::from_secs(1);
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

struct Wire<'a> {
    ws: WebSocketStream<MaybeTlsStream<TcpStream>>,
    room: &'a str,
    peer: &'a str,
    seq: u64,
    keys: Vec<Value>,
    decline_for: Option<String>,
}
impl Wire<'_> {
    async fn frame(&mut self) -> Result<Value> {
        loop {
            match self.ws.next().await {
                Some(Ok(Message::Text(raw))) => {
                    let f: Value =
                        serde_json::from_str(&raw).map_err(|_| "Invalid call signaling.")?;
                    if f["command"] == "client_state" && f["data"]["is_verified"] != true {
                        return Err("The verified call session ended.");
                    }
                    if f["command"] == "sync_complete" {
                        for e in crate::rooms::items(&f["data"]["to_device"]) {
                            if e["type"] == KEY
                                && e["encrypted"] == true
                                && e["sender"] == self.peer
                                && e["content"]["room_id"] == self.room
                            {
                                if self.keys.len() >= 64 {
                                    return Err("Too many pending call keys.");
                                }
                                self.keys.push(e["content"].clone());
                            }
                        }
                        for e in crate::rooms::items(&f["data"]["rooms"][self.room]["events"]) {
                            if e["sender"] == self.peer
                                && (e["type"] == "org.matrix.msc4310.rtc.decline"
                                    || e["decrypted_type"] == "org.matrix.msc4310.rtc.decline")
                            {
                                let c = if e["decrypted"].is_object() {
                                    &e["decrypted"]
                                } else {
                                    &e["content"]
                                };
                                self.decline_for =
                                    c["m.relates_to"]["event_id"].as_str().map(str::to_owned);
                            }
                        }
                    }
                    return Ok(f);
                }
                Some(Ok(Message::Ping(_) | Message::Pong(_))) => continue,
                _ => return Err("Lost the Matrix call connection."),
            }
        }
    }
    async fn rpc(&mut self, name: &str, body: Value) -> Result<Value> {
        time::timeout(SECOND * 10, async {
            self.seq += 1;
            self.ws
                .send(Message::Text(
                    json!({"command":name,"request_id":self.seq,"data":body})
                        .to_string()
                        .into(),
                ))
                .await
                .map_err(|_| "Matrix call signaling failed.")?;
            loop {
                let f = self.frame().await?;
                if f["request_id"] == self.seq {
                    match f["command"].as_str() {
                        Some("response") => return Ok(f["data"].clone()),
                        Some("error") => return Err("Homeserver rejected the call request."),
                        _ => (),
                    }
                }
            }
        })
        .await
        .map_err(|_| "Matrix call request timed out.")?
    }
    async fn state(&mut self, initial: bool) -> Result<Vec<Value>> {
        let data=self.rpc("get_room_state",json!({"room_id":self.room,"fetch_members":initial,"refetch":initial,"include_members":true})).await?;
        data.as_array().cloned().ok_or("Room state unavailable.")
    }
    async fn open_slot(&mut self, state: &[Value]) -> Result<()> {
        let previous = state
            .iter()
            .find(|e| e["type"] == SLOT && e["state_key"] == SLOT_ID)
            .map(|e| &e["content"]);
        let mut content = previous.cloned().unwrap_or_else(|| json!({}));
        if !content.is_object() {
            return Err("Invalid call slot state.");
        }
        content["application"] = json!({"type":"m.call"});
        content["status"] = json!("open");
        content["encryption"] = json!({"type":"org.matrix.msc4143.per_member"});
        if previous == Some(&content) {
            return Ok(());
        }
        self.rpc(
            "set_state",
            json!({"room_id":self.room,"type":SLOT,"state_key":SLOT_ID,"content":content}),
        )
        .await?;
        Ok(())
    }
}
impl untethered_iced::rtc_membership::Signal for &mut Wire<'_> {
    async fn request(
        &mut self,
        cmd: &'static str,
        body: Value,
    ) -> std::result::Result<Value, String> {
        self.rpc(cmd, body).await.map_err(str::to_owned)
    }
}
fn allowed(url: &Url, tls: &str, local: &str, allow_loopback: bool) -> bool {
    url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && (url.scheme() == tls
            || (allow_loopback
                && url.scheme() == local
                && matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"))))
}
fn focus(list: &Value) -> Option<Url> {
    list.as_array()?
        .iter()
        .filter(|t| t["type"] == "livekit")
        .find_map(|t| Url::parse(t["livekit_service_url"].as_str()?).ok())
}
fn check_dm(state: &[Value], user: &str, peer: &str) -> Result<()> {
    let joined: Vec<_> = state
        .iter()
        .filter(|e| e["type"] == "m.room.member" && e["content"]["membership"] == "join")
        .filter_map(|e| e["state_key"].as_str())
        .collect();
    if user == peer
        || joined.len() != 2
        || !joined.contains(&user)
        || !joined.contains(&peer)
        || !state.iter().any(|e| {
            e["type"] == "m.room.encryption" && e["content"]["algorithm"] == "m.megolm.v1.aes-sha2"
        })
    {
        return Err("Voice calls require an encrypted DM with two joined people.");
    }
    Ok(())
}
fn active_member(e: &Value) -> bool {
    let c = &e["content"];
    e["type"] == EVENT_TYPE
        && c["application"] == "m.call"
        && c["scope"] == "m.room"
        && c["call_id"] == ""
        && c["device_id"].is_string()
        && c["created_ts"]
            .as_u64()
            .or(e["timestamp"].as_u64())
            .unwrap_or(0)
            .saturating_add(c["expires"].as_u64().unwrap_or(14_400_000))
            > now()
}
struct Session {
    user: String,
    device: String,
    focus: Url,
    sfu: String,
    token: String,
}
async fn prepare(w: &mut Wire<'_>) -> Result<Session> {
    let identity = w.rpc("get_state", json!({})).await?;
    if identity["is_verified"] != true {
        return Err("Verify the Matrix device before calling.");
    }
    let user = identity["user_id"].as_str().ok_or("No Matrix user.")?;
    let device = identity["device_id"].as_str().ok_or("No Matrix device.")?;
    let state = w.state(true).await?;
    check_dm(&state, user, w.peer)?;
    if state.iter().any(active_member) {
        return Err("A call is already active in this DM.");
    }
    let hs = Url::parse(identity["homeserver_url"].as_str().unwrap_or(""))
        .map_err(|_| "Unknown homeserver.")?;
    let local = matches!(hs.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    let http = reqwest::Client::builder()
        .timeout(SECOND * 7)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| "Could not create RTC client.")?;
    let mut service = focus(
        &w.rpc("get_rtc_transports", json!({}))
            .await
            .unwrap_or_default()["rtc_transports"],
    );
    if service.is_none() {
        // Legacy Element Call deployments announce their OpenID service here.
        let canonical = user
            .split_once(':')
            .and_then(|(_, s)| Url::parse(&format!("https://{s}")).ok());
        for base in std::iter::once(hs).chain(canonical) {
            if !allowed(&base, "https", "http", local) {
                continue;
            }
            let Ok(response) = http
                .get(base.join("/.well-known/matrix/client").unwrap())
                .send()
                .await
            else {
                continue;
            };
            if let Ok(body) = response.json::<Value>().await {
                service = focus(&body["org.matrix.msc4143.rtc_foci"]);
            }
            if service.is_some() {
                break;
            }
        }
    }
    let focus = service
        .filter(|s| allowed(s, "https", "http", local))
        .ok_or("No supported MatrixRTC LiveKit service advertised.")?;
    let openid = w.rpc("request_openid_token", json!({})).await?;
    let auth: Value = http
        .post(format!("{}/sfu/get", focus.as_str().trim_end_matches('/')))
        .json(&json!({"room":w.room,"device_id":device,"openid_token":openid}))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| "Call server authorization failed.")?
        .json()
        .await
        .map_err(|_| "Invalid call server response.")?;
    let sfu = Url::parse(auth["url"].as_str().unwrap_or("")).map_err(|_| "Invalid SFU URL.")?;
    if !allowed(&sfu, "wss", "ws", local) {
        return Err("The SFU requires a secure connection.");
    }
    let token = auth["jwt"]
        .as_str()
        .ok_or("Missing SFU authorization.")?
        .to_owned();
    w.open_slot(&state).await?;
    Ok(Session {
        user: user.into(),
        device: device.into(),
        focus,
        sfu: sfu.into(),
        token,
    })
}
async fn status(events: &Sender<Value>, room: &str, label: &str) {
    let _ = events
        .send(json!({"command":"desktop_call","data":{"room":room,"label":label}}))
        .await;
}
pub async fn run(
    addr: SocketAddr,
    cookie: &str,
    room: &str,
    peer: &str,
    cancel: Receiver<()>,
    events: &Sender<Value>,
) -> Result<&'static str> {
    let mut req = format!("ws://{addr}/_gomuks/websocket")
        .into_client_request()
        .map_err(|_| "Cannot open call signaling.")?;
    req.headers_mut().insert(
        "Cookie",
        cookie.parse().map_err(|_| "Missing Matrix session.")?,
    );
    let ws = tokio_tungstenite::connect_async(req)
        .await
        .map_err(|_| "Could not connect to gomuks.")?
        .0;
    let mut w = Wire {
        ws,
        room,
        peer,
        seq: 0,
        keys: vec![],
        decline_for: None,
    };
    let s = tokio::select! {biased; _=cancel.recv()=>return Ok(ENDED),s=prepare(&mut w)=>s?};
    if cancel.is_closed() {
        return Ok(ENDED);
    }
    let mut member = Membership::join(room, &s.user, &s.device, &s.focus, 16_000, &mut w)
        .await
        .map_err(|_| "Could not join the MatrixRTC call.")?;
    let prior = w.rpc("listen_to_device", json!(true)).await;
    let mut media = None;
    let result = if prior.is_ok() {
        tokio::select! {biased;_=cancel.recv()=>Ok(ENDED),r=call(&mut w,&s,&mut member,events,&mut media)=>r}
    } else {
        Err("Could not enable encrypted call signaling.")
    };
    if let Some(lk) = media {
        let _ = time::timeout(SECOND * 4, lk.close()).await;
    }
    // Dropping call() already stopped the audio jobs. Publish leave even on error/cancel.
    let leave = member.leave(&mut w).await;
    if let Ok(old) = prior {
        let _ = w.rpc("listen_to_device", old).await;
    }
    leave.map_err(|_| "Disconnected; homeserver will expire call membership.")?;
    result
}
async fn call(
    w: &mut Wire<'_>,
    s: &Session,
    m: &mut Membership,
    events: &Sender<Value>,
    media: &mut Option<Room>,
) -> Result<&'static str> {
    let notification = w
        .rpc(
            "send_event",
            json!({"room_id":w.room,"type":RING,"synchronous":true,"content":{
        "slot_id":SLOT_ID,"notification_type":"ring","m.mentions":{"user_ids":[w.peer]},
        "m.relates_to":{"rel_type":"m.reference","event_id":m.event_id},
        "m.call.intent":"audio","sender_ts":now(),"lifetime":45_000,"msc4354_sticky_key":SLOT_ID}}),
        )
        .await?;
    status(events, w.room, "Ringing").await;
    let mut renew = Instant::now();
    let mut heartbeat = time::interval(SECOND * 4);
    let mut poll = time::interval(SECOND);
    let answer=time::timeout(SECOND*45,async {
        loop {
            tokio::select!{
                _=heartbeat.tick()=>m.heartbeat(&mut *w).await.map_err(|_|"Lost call membership.")?,
                _=poll.tick()=>{
                    let state=w.state(false).await?;check_dm(&state,&s.user,w.peer)?;
                    if let Some(e)=state.iter().find(|e|e["sender"]==w.peer && active_member(e)){
                        let c=&e["content"];let device=c["device_id"].as_str().unwrap();
                        let id=format!("{}:{device}",w.peer);
                        return Ok((device.to_owned(),c["membershipID"].as_str().unwrap_or(&id).to_owned(),id));
                    }
                }
            }
            if w.decline_for.as_deref().is_some_and(|id|notification["event_id"]==id){return Err("Call declined.")}
        }
    }).await.map_err(|_|"No answer.")??;
    status(events, w.room, "Connecting").await;
    let (peer_device, peer_member, peer_id) = answer;
    let provider = rtc_keys::provider();
    let my_id = format!("{}:{}", s.user, s.device);
    let key = SenderKey::fresh(0)?;
    w.rpc(
        "send_to_device",
        key.to_device(w.room, &my_id, &s.device, w.peer, &peer_device),
    )
    .await?;
    fn install(
        w: &mut Wire<'_>,
        p: &KeyProvider,
        device: &str,
        member: &str,
        id: &str,
    ) -> Result<bool> {
        let mut installed = false;
        for c in w.keys.drain(..) {
            let Some((idx, bytes)) = rtc_keys::receive(&c, device, member)? else {
                continue;
            };
            if !p.set_key(&id.to_owned().into(), idx, bytes) {
                return Err("Failed to install sender key.");
            }
            installed = true;
        }
        Ok(installed)
    }
    time::timeout(SECOND*14,async {
        while !install(w,&provider,&peer_device,&peer_member,&peer_id)? {
            tokio::select!{r=w.frame()=>{r?;},_=heartbeat.tick()=>m.heartbeat(&mut *w).await.map_err(|_|"Lost call membership.")?}
        }
        Ok::<(),&'static str>(())
    }).await.map_err(|_|"Element did not provide an encrypted audio key.")??;
    if !provider.set_key(&my_id.clone().into(), 0, key.bytes.to_vec()) {
        return Err("Failed to install local key.");
    }
    let mut options = RoomOptions::default();
    options.encryption = Some(E2eeOptions {
        encryption_type: EncryptionType::Gcm,
        key_provider: provider.clone(),
    });
    let (lk, mut rtc) = time::timeout(SECOND * 12, Room::connect(&s.sfu, &s.token, options))
        .await
        .map_err(|_| "SFU connection timed out.")?
        .map_err(|_| "Could not connect to SFU.")?;
    let lk = media.insert(lk);
    if lk.local_participant().identity().0 != my_id {
        return Err("SFU assigned an unexpected identity.");
    }
    let src = NativeAudioSource::new(AudioSourceOptions::default(), 48000, 1, 50);
    let mic =
        LocalAudioTrack::create_audio_track("Microphone", RtcAudioSource::Native(src.clone()));
    let publication = lk
        .local_participant()
        .publish_track(
            LocalTrack::Audio(mic),
            TrackPublishOptions {
                source: TrackSource::Microphone,
                ..Default::default()
            },
        )
        .await
        .map_err(|_| "Could not publish microphone.")?;
    if publication.encryption_type() != EncryptionType::Gcm {
        return Err("Microphone encryption unavailable.");
    }
    let mut audio = JoinSet::new();
    audio.spawn(capture(src));
    let mut playing = false;
    loop {
        install(w, &provider, &peer_device, &peer_member, &peer_id)?;
        tokio::select! {
            e=w.frame()=>{e?;}
            _=heartbeat.tick()=>{
                if renew.elapsed()>SECOND*36 {m.renew(&mut *w).await.map_err(|_|"Failed to renew call membership.")?;renew=Instant::now();}
                else {m.heartbeat(&mut *w).await.map_err(|_|"Lost call membership.")?;}
                let state=w.state(false).await?;check_dm(&state,&s.user,w.peer)?;
                if !state.iter().any(|e|e["sender"]==w.peer && e["content"]["device_id"]==peer_device && active_member(e)){return Ok(ENDED)}
            }
            job=audio.join_next()=>return Err(job.and_then(|j|j.ok()).and_then(|v|v.err()).unwrap_or("Audio device stopped.")),
            event=rtc.recv()=>match event {
                Some(RoomEvent::TrackSubscribed{track:RemoteTrack::Audio(track),participant,publication,..}) if participant.identity().0==peer_id && !playing=>{
                    if publication.encryption_type()!=EncryptionType::Gcm{return Err("Peer did not encrypt audio.")}
                    audio.spawn(play(track));playing=true;status(events,w.room,"Connected").await;
                }
                Some(RoomEvent::ParticipantDisconnected(p)) if p.identity().0==peer_id=>return Ok(ENDED),
                None|Some(RoomEvent::Disconnected{..}|RoomEvent::Reconnecting)=>return Err("Call connection lost."),
                _=>()
            }
        }
    }
}

fn audio_process(record: bool) -> Result<tokio::process::Child> {
    let mut p = Command::new("pacat");
    p.args([
        if record { "--record" } else { "--playback" },
        "--raw",
        "--format=s16le",
        "--rate=48000",
        "--channels=1",
        "--latency-msec=40",
        "--client-name=Untethered",
        "--stream-name=Voice call",
    ])
    .kill_on_drop(true)
    .stderr(Stdio::null());
    if record {
        p.stdin(Stdio::null()).stdout(Stdio::piped());
    } else {
        p.stdin(Stdio::piped()).stdout(Stdio::null());
    }
    p.spawn()
        .map_err(|_| "Cannot open audio. Install the PulseAudio client tools.")
}
async fn capture(src: NativeAudioSource) -> Result<()> {
    let mut child = audio_process(true)?;
    let mut input = child.stdout.take().unwrap();
    let mut buf = [0u8; 960];
    loop {
        input
            .read_exact(&mut buf)
            .await
            .map_err(|_| "Microphone unavailable.")?;
        let pcm: Vec<_> = buf
            .as_chunks::<2>()
            .0
            .iter()
            .copied()
            .map(i16::from_le_bytes)
            .collect();
        src.capture_frame(&AudioFrame {
            data: pcm.into(),
            sample_rate: 48000,
            num_channels: 1,
            samples_per_channel: 480,
        })
        .await
        .map_err(|_| "Microphone capture failed.")?;
    }
}
async fn play(track: RemoteAudioTrack) -> Result<()> {
    let mut child = audio_process(false)?;
    let mut out = child.stdin.take().unwrap();
    let mut stream = NativeAudioStream::new(track.rtc_track(), 48000, 1);
    while let Some(frame) = stream.next().await {
        let bytes: Vec<_> = frame.data.iter().flat_map(|s| s.to_le_bytes()).collect();
        out.write_all(&bytes)
            .await
            .map_err(|_| "Speaker unavailable.")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_direct_chat_marker_cannot_ring_a_group_or_clear_room() {
        let member = |id: &str| json!({"type":"m.room.member","state_key":id,"content":{"membership":"join"}});
        let mut state = vec![member("@a:chat"), member("@b:chat")];
        assert!(check_dm(&state, "@a:chat", "@b:chat").is_err());
        state.push(
            json!({"type":"m.room.encryption","content":{"algorithm":"m.megolm.v1.aes-sha2"}}),
        );
        assert!(check_dm(&state, "@a:chat", "@b:chat").is_ok());
        assert!(check_dm(&state, "@a:chat", "@c:chat").is_err());
        state.push(member("@c:chat"));
        assert!(check_dm(&state, "@a:chat", "@b:chat").is_err());
    }
}
