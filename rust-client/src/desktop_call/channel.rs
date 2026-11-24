//! The room-wide voice slot is entered explicitly, even by its first member.
//! Its lifetime is local: a remote unpublish/leave stops only that peer's audio.
use super::*;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use tokio::task::AbortHandle;

#[derive(Clone, PartialEq)]
struct Recipient {
    user: String,
    device: String,
    member: String,
    created: u64,
}
type Roster = BTreeMap<String, Recipient>;

pub(super) fn check_room(state: &[Value], user: &str) -> Result<()> {
    let has = |kind: &str| {
        state
            .iter()
            .find(|e| e["type"] == kind && e["state_key"] == "")
    };
    if !has("m.room.create").is_some_and(|e| e["content"]["type"] == crate::rooms::VOICE_ROOM)
        || !has("m.room.encryption")
            .is_some_and(|e| e["content"]["algorithm"] == "m.megolm.v1.aes-sha2")
        || !state.iter().any(|e| {
            e["type"] == "m.room.member"
                && e["state_key"] == user
                && e["content"]["membership"] == "join"
        })
    {
        return Err("Join an encrypted voice room before connecting.");
    }
    Ok(())
}

fn roster(state: &[Value], local: &str, service: &Url) -> Roster {
    let joined: BTreeSet<_> = state
        .iter()
        .filter(|e| e["type"] == "m.room.member" && e["content"]["membership"] == "join")
        .filter_map(|e| e["state_key"].as_str())
        .collect();
    state
        .iter()
        .filter(|e| active_member(e))
        .filter_map(|e| {
            let user = e["sender"].as_str()?;
            let c = &e["content"];
            let device = c["device_id"].as_str()?;
            let identity = format!("{user}:{device}");
            // A room member can write state with arbitrary content. Do not use
            // another user's claimed identity or expand a to-device wildcard.
            if identity == local
                || !joined.contains(user)
                || device.is_empty()
                || device == "*"
                || e["state_key"] != format!("_{user}_{device}_m.call")
                || !crate::rooms::items(&c["foci_preferred"]).any(|f| {
                    f["type"] == "livekit"
                        && f["livekit_service_url"].as_str().is_some_and(|u| {
                            u.trim_end_matches('/') == service.as_str().trim_end_matches('/')
                        })
                })
            {
                return None;
            }
            let member = c["membershipID"].as_str().unwrap_or(&identity).to_owned();
            Some((
                identity,
                Recipient {
                    user: user.into(),
                    device: device.into(),
                    member,
                    created: created(e)?,
                },
            ))
        })
        .collect()
}

struct GroupKeys {
    provider: KeyProvider,
    local: String,
    outbound: SenderKey,
    members: Roster,
    ready: BTreeSet<String>,
}
impl GroupKeys {
    fn new(session: &Session) -> Result<Self> {
        let local = format!("{}:{}", session.user, session.device);
        let outbound = SenderKey::fresh(0)?;
        let provider = rtc_keys::provider();
        if !provider.set_key(&local.clone().into(), 0, outbound.bytes.to_vec()) {
            return Err("Cannot install voice room key.");
        }
        Ok(Self {
            provider,
            local,
            outbound,
            members: Roster::new(),
            ready: BTreeSet::new(),
        })
    }
    async fn reconcile(
        &mut self,
        w: &mut Wire<'_>,
        s: &Session,
        next: Roster,
        lk: &Room,
        mic: &LocalTrackPublication,
    ) -> Result<()> {
        if next == self.members {
            return Ok(());
        }
        self.ready
            .retain(|id| next.contains_key(id) && next.get(id) == self.members.get(id));
        // Fresh, independent key for each roster change, including departures.
        // Select it on the existing publication *before* distributing it: do
        // not publish new frames for a departed device with the old key.
        let key = SenderKey::fresh((self.outbound.index + 1) % 255)?;
        let local = lk.local_participant().identity();
        let cryptor = lk
            .e2ee_manager()
            .frame_cryptors()
            .remove(&(local.clone(), mic.sid()))
            .ok_or("Missing voice room microphone cryptor.")?;
        if !cryptor.enabled()
            || !self
                .provider
                .set_key(&local, key.index.into(), key.bytes.to_vec())
        {
            return Err("Cannot rotate voice room key.");
        }
        cryptor.set_key_index(key.index.into());
        if cryptor.key_index() != i32::from(key.index) {
            return Err("Cannot select voice room key.");
        }
        if !next.is_empty() {
            let request = key.to_devices(
                w.room,
                &self.local,
                &s.device,
                next.values().map(|p| (p.user.as_str(), p.device.as_str())),
            );
            w.rpc("send_to_device", request).await?;
        }
        self.outbound = key;
        self.members = next;
        Ok(())
    }
    fn receive(&mut self, w: &mut Wire<'_>) -> Result<()> {
        let mut early = Vec::new();
        for (received, e) in w.keys.drain(..) {
            let c = &e["content"];
            let (Some(user), Some(device)) = (
                e["sender"].as_str(),
                c["member"]["claimed_device_id"].as_str(),
            ) else {
                continue;
            };
            let id = format!("{user}:{device}");
            let Some(peer) = self.members.get(&id) else {
                // The Olm sync can beat the matching room-state sync.
                if received.elapsed() < SECOND * 8 {
                    early.push((received, e))
                }
                continue;
            };
            if peer.user != user || peer.device != device {
                continue;
            }
            if let Some((index, bytes)) = rtc_keys::receive(c, device, &peer.member)? {
                if !self.provider.set_key(&id.clone().into(), index, bytes) {
                    return Err("Cannot install participant key.");
                }
                self.ready.insert(id);
            }
        }
        w.keys = early;
        Ok(())
    }
}

#[derive(Default)]
struct Speakers {
    jobs: JoinSet<Result<()>>,
    tracks: HashMap<TrackSid, AbortHandle>,
}
impl Speakers {
    fn sync(
        &mut self,
        lk: &Room,
        keys: &GroupKeys,
        mode: &watch::Receiver<AudioMode>,
        notices: &DeviceNotices,
    ) -> Result<()> {
        let mut live = BTreeSet::new();
        for (id, p) in lk.remote_participants() {
            for (sid, publication) in p.track_publications() {
                let wanted = keys.ready.contains(&id.0)
                    && publication.kind() == TrackKind::Audio
                    && publication.source() == TrackSource::Microphone;
                if wanted && publication.encryption_type() != EncryptionType::Gcm {
                    return Err("Unencrypted voice room microphone.");
                }
                if publication.is_desired() != wanted {
                    publication.set_subscribed(wanted);
                }
                if wanted && let Some(RemoteTrack::Audio(track)) = publication.track() {
                    live.insert(sid.clone());
                    self.tracks.entry(sid).or_insert_with(|| {
                        self.jobs.spawn(play(track, mode.clone(), notices.clone()))
                    });
                }
            }
        }
        self.tracks.retain(|sid, job| {
            if live.contains(sid) {
                true
            } else {
                job.abort();
                false
            }
        });
        Ok(())
    }
}

pub(super) async fn joined(
    w: &mut Wire<'_>,
    s: &Session,
    m: &mut Membership,
    events: &Sender<Value>,
    media: &mut Option<Room>,
    mode: watch::Receiver<AudioMode>,
) -> Result<&'static str> {
    let mut keys = GroupKeys::new(s)?;
    let (lk, mut rtc, src, mic) = publish_mic(s, &keys.provider, media, false).await?;
    let notices = DeviceNotices {
        events: events.clone(),
        room: w.room.to_owned(),
    };
    // Keep capture paced by the audio device while membership and key RPCs
    // are in flight. Dropping either task set releases its pacat children.
    let mut input = JoinSet::new();
    input.spawn(controlled_capture(
        src,
        mic.clone(),
        mode.clone(),
        notices.clone(),
    ));
    let mut speakers = Speakers::default();
    let mut heartbeat = time::interval(SECOND * 4);
    heartbeat.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
    let mut renew = Instant::now();
    let mut label = String::new();
    w.roster_dirty = true;
    loop {
        if std::mem::take(&mut w.roster_dirty) {
            let state = w.state(false).await?;
            check_room(&state, &s.user)?;
            keys.reconcile(w, s, roster(&state, &keys.local, &s.focus), lk, &mic)
                .await?;
            let next = format!("In voice · {} connected", keys.members.len() + 1);
            if label != next {
                status(events, w.room, &next).await;
                label = next;
            }
        }
        keys.receive(w)?;
        speakers.sync(lk, &keys, &mode, &notices)?;
        tokio::select! {
            r=input.join_next()=>return Err(r.and_then(|j|j.ok()).and_then(|v|v.err()).unwrap_or("Microphone stream ended.")),
            job=speakers.jobs.join_next(),if !speakers.jobs.is_empty()=>match job {
                Some(Ok(result))=>result?,
                Some(Err(e)) if e.is_cancelled()=>(),
                _=>return Err("Voice room playback failed."),
            },
            frame=w.frame()=>{frame?;}
            _=heartbeat.tick()=>{
                if renew.elapsed()>SECOND*36{m.renew(&mut *w).await.map_err(|_|"Could not renew voice room membership.")?;renew=Instant::now();}
                else{m.heartbeat(&mut *w).await.map_err(|_|"Lost voice room membership.")?;}
                w.roster_dirty=true;
            }
            e=rtc.recv()=>match e {
                Some(RoomEvent::Reconnecting|RoomEvent::Disconnected{..})|None=>return Err("Voice room connection lost."),
                // Remote joins, subscriptions and leaves are reconciled independently.
                _=>(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_only_go_to_joined_users_active_devices_in_this_slot() {
        let focus = Url::parse("https://rtc.example/").unwrap();
        let user = "@other:example";
        let joined =
            json!({"type":"m.room.member","state_key":user,"content":{"membership":"join"}});
        let device = |d: &str| {
            json!({"type":EVENT_TYPE,"sender":user,"timestamp":now(),
            "state_key":format!("_{user}_{d}_m.call"),"content":{"application":"m.call","scope":"m.room",
            "call_id":"","device_id":d,"expires":60000,"foci_preferred":[{"type":"livekit","livekit_service_url":focus.as_str()}]}})
        };
        let mut expired = device("E");
        expired["timestamp"] = 1.into();
        let mut wrong_slot = device("W");
        wrong_slot["content"]["call_id"] = "another".into();
        let mut spoofed = device("S");
        spoofed["state_key"] = "_@someone:example_S_m.call".into();
        let mut unjoined = device("X");
        unjoined["sender"] = "@left:example".into();
        unjoined["state_key"] = "_@left:example_X_m.call".into();
        let mut other_sfu = device("F");
        other_sfu["content"]["foci_preferred"][0]["livekit_service_url"] =
            "https://elsewhere.example/".into();
        let state = vec![
            joined,
            device("A"),
            device("B"),
            device("*"),
            expired,
            wrong_slot,
            spoofed,
            unjoined,
            other_sfu,
        ];
        let recipients = roster(&state, "@me:example:D", &focus);
        assert_eq!(recipients.len(), 2);
        let key = SenderKey::fresh(0).unwrap();
        let cmd = key.to_devices(
            "!room",
            "@me:example:D",
            "D",
            recipients
                .values()
                .map(|p| (p.user.as_str(), p.device.as_str())),
        );
        assert_eq!(cmd["encrypted"], true);
        assert_eq!(cmd["messages"].as_object().unwrap().len(), 1);
        assert_eq!(
            cmd["messages"][user]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["A", "B"])
        );
    }
}
