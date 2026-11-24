//! Window-only ScreenCast portal. The PipeWire client receives the session's
//! restricted FD and node, never a desktop-wide/default capture source.
use super::*;
use ashpd::desktop::{
    PersistMode, Session as PortalSession,
    screencast::{CursorMode, Screencast, SourceType},
};
use iced::widget::image::Handle;
use livekit::webrtc::{
    prelude::{
        I420Buffer, RtcVideoSource, VideoFormatType, VideoFrame, VideoResolution, VideoRotation,
    },
    video_source::native::NativeVideoSource,
    video_stream::native::NativeVideoStream,
};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use tokio::process::{Child, ChildStdout};
use uuid::Uuid;

const W: u32 = 960;
const H: u32 = 540;
const Y: usize = (W * H) as usize;
const FRAME: usize = Y * 3 / 2;

type Tiles = BTreeMap<String, (String, Handle)>;
#[derive(Clone, Default)]
pub(crate) struct Previews(Arc<Mutex<Tiles>>);
impl Previews {
    pub(crate) fn tiles(&self) -> Vec<(String, Handle)> {
        self.0.lock().unwrap().values().cloned().collect()
    }
}
struct Tile {
    sid: String,
    views: Previews,
    events: Sender<Value>,
}
impl Drop for Tile {
    fn drop(&mut self) {
        self.views.0.lock().unwrap().remove(&self.sid);
        let _ = self.events.try_send(json!({"command":"video_redraw"}));
    }
}
pub(super) async fn display(
    track: RemoteVideoTrack,
    user: String,
    views: Previews,
    events: Sender<Value>,
) -> Result<()> {
    let tile = Tile {
        sid: track.sid().to_string(),
        views,
        events,
    };
    let mut stream = NativeVideoStream::new(track.rtc_track());
    let mut tick = time::interval(Duration::from_millis(100));
    tick.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let Some(frame) = stream.next().await else {
            return Ok(());
        };
        let (w, h) = (frame.buffer.width(), frame.buffer.height());
        if w == 0 || h == 0 || u64::from(w) * u64::from(h) > 8_294_400 {
            continue;
        }
        let mut buf = frame.buffer.to_i420();
        let scale = (W as f64 / w as f64).min(H as f64 / h as f64).min(1.);
        let (ow, oh) = (
            (f64::from(w) * scale).round().max(1.) as u32,
            (f64::from(h) * scale).round().max(1.) as u32,
        );
        if (ow, oh) != (w, h) {
            buf = buf.scale(ow as i32, oh as i32);
        }
        let mut rgba = vec![0; (ow * oh * 4) as usize];
        use livekit::webrtc::video_frame::native::VideoFrameBufferExt;
        buf.to_argb(
            VideoFormatType::ABGR,
            &mut rgba,
            ow * 4,
            ow as i32,
            oh as i32,
        );
        tile.views.0.lock().unwrap().insert(
            tile.sid.clone(),
            (user.clone(), Handle::from_rgba(ow, oh, rgba)),
        );
        let _ = tile.events.try_send(json!({"command":"video_redraw"}));
    }
}

type Session = PortalSession<'static, Screencast<'static>>;
struct Lease(Option<Session>);
impl Lease {
    async fn close(&mut self) {
        if let Some(session) = &self.0 {
            let _ = session.close().await;
            self.0 = None;
        }
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(session) = self.0.take() {
            tokio::spawn(async move {
                let _ = session.close().await;
            });
        }
    }
}
pub(super) struct Capture {
    lease: Lease,
    child: Child,
    pipe: ChildStdout,
    pixels: Vec<u8>,
}
impl Capture {
    async fn choose() -> Result<Self> {
        let proxy = Screencast::new()
            .await
            .map_err(|_| "Window capture portal unavailable.")?;
        let lease = Lease(Some(
            proxy
                .create_session()
                .await
                .map_err(|_| "Could not open window picker.")?,
        ));
        let session = lease.0.as_ref().unwrap();
        proxy
            .select_sources(
                session,
                CursorMode::Hidden,
                SourceType::Window.into(),
                false,
                None,
                PersistMode::DoNot,
            )
            .await
            .and_then(|r| r.response())
            .map_err(|_| "The desktop could not select a window.")?;
        let selection = proxy
            .start(session, None)
            .await
            .and_then(|r| r.response())
            .map_err(|_| "Window sharing was cancelled or denied.")?;
        let [stream] = selection.streams() else {
            return Err("Select exactly one window.");
        };
        if stream
            .source_type()
            .is_some_and(|t| t != SourceType::Window)
        {
            return Err("The selection was not a window.");
        }
        let fd = proxy
            .open_pipe_wire_remote(session)
            .await
            .map_err(|_| "Could not open the selected PipeWire stream.")?;
        // Stdio maps the already-authorized FD to descriptor 0 in the child;
        // no inheritance of arbitrary FDs, temporary recording or shell needed.
        let mut child = Command::new("gst-launch-1.0")
            .args([
                "-q",
                "pipewiresrc",
                "fd=0",
                &format!("path={}", stream.pipe_wire_node_id()),
                "keepalive-time=300",
                "do-timestamp=true",
                "!",
                "queue",
                "max-size-buffers=2",
                "leaky=downstream",
                "!",
                "videorate",
                "drop-only=true",
                "max-rate=12",
                "!",
                "videoconvert",
                "!",
                "videoscale",
                "add-borders=true",
                "!",
                "video/x-raw,format=I420,width=960,height=540,pixel-aspect-ratio=1/1",
                "!",
                "fdsink",
                "fd=1",
                "sync=false",
            ])
            .stdin(Stdio::from(fd))
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| "Install GStreamer and its PipeWire plugin to share a window.")?;
        let mut pipe = child.stdout.take().unwrap();
        let mut pixels = vec![0; FRAME];
        time::timeout(SECOND * 12, pipe.read_exact(&mut pixels))
            .await
            .map_err(|_| "Window capture timed out.")?
            .map_err(|_| "Could not read the selected window.")?;
        Ok(Self {
            lease,
            child,
            pipe,
            pixels,
        })
    }
    async fn feed(
        mut self,
        src: NativeVideoSource,
        mut mode: watch::Receiver<AudioMode>,
        id: Uuid,
    ) -> Result<()> {
        // Stop capture independently of room-state and key-distribution RPCs.
        let result = tokio::select! {
            biased;
            _=mode.wait_for(|m|m.share!=Some(id))=>Ok(()),
            f=self.pump(src)=>f,
        };
        let _ = self.child.kill().await;
        self.lease.close().await;
        result
    }
    async fn pump(&mut self, src: NativeVideoSource) -> Result<()> {
        let closed = self
            .lease
            .0
            .as_ref()
            .unwrap()
            .receive_closed()
            .await
            .map_err(|_| "Window capture ended.")?;
        tokio::pin!(closed);
        loop {
            let mut buf = I420Buffer::new(W, H);
            let (y, u, v) = buf.data_mut();
            y.copy_from_slice(&self.pixels[..Y]);
            u.copy_from_slice(&self.pixels[Y..Y + Y / 4]);
            v.copy_from_slice(&self.pixels[Y + Y / 4..]);
            src.capture_frame(&VideoFrame::new(VideoRotation::VideoRotation0, buf));
            tokio::select! {
                _=closed.next()=>return Ok(()),
                f=time::timeout(SECOND*12,self.pipe.read_exact(&mut self.pixels))=>{f.map_err(|_|"Window capture timed out.")?.map_err(|_|"Window capture ended.")?;}
            }
        }
    }
}

#[derive(Default)]
pub(super) struct Share {
    request: Option<Uuid>,
    picker: JoinSet<Result<Capture>>,
    frames: JoinSet<Result<()>>,
    published: Option<TrackSid>,
}
pub(super) enum Update {
    Chosen(Result<Box<Capture>>),
    Ended(&'static str),
}
impl Share {
    pub(super) async fn select(&mut self, request: Option<Uuid>, lk: &Room) -> Result<()> {
        if self.request == request {
            return Ok(());
        }
        self.stop(lk).await?;
        self.request = request;
        if request.is_some() {
            self.picker.spawn(Capture::choose());
        }
        Ok(())
    }
    async fn stop(&mut self, lk: &Room) -> Result<()> {
        self.picker.shutdown().await;
        // On an explicit stop the worker observes the same watch change and
        // closes the portal session itself. Abort on timeout or call teardown;
        // Lease::drop and Child::kill_on_drop cover those paths.
        if !self.frames.is_empty() {
            let _ = time::timeout(SECOND, self.frames.join_next()).await;
        }
        self.frames.shutdown().await;
        if let Some(sid) = self.published.take() {
            lk.local_participant()
                .unpublish_track(&sid)
                .await
                .map_err(|_| "Could not unpublish window share.")?;
        }
        Ok(())
    }
    pub(super) async fn next(&mut self) -> Update {
        tokio::select! {
            p=self.picker.join_next(),if !self.picker.is_empty()=>Update::Chosen(p.and_then(|r|r.ok()).unwrap_or(Err("Window selection interrupted.")).map(Box::new)),
            f=self.frames.join_next(),if !self.frames.is_empty()=>Update::Ended(f.and_then(|r|r.ok()).and_then(|r|r.err()).unwrap_or("Window sharing stopped.")),
            else=>std::future::pending().await,
        }
    }
    pub(super) async fn update(
        &mut self,
        update: Update,
        lk: &Room,
        index: u8,
        events: &Sender<Value>,
        room: &str,
        mode: &watch::Receiver<AudioMode>,
    ) -> Result<()> {
        let Some(id) = self.request else {
            return Ok(());
        };
        if mode.borrow().share != Some(id) {
            return Ok(());
        }
        let outcome = match update {
            Update::Chosen(Ok(cap)) => {
                let src = NativeVideoSource::new(
                    VideoResolution {
                        width: W,
                        height: H,
                    },
                    true,
                );
                let video = LocalVideoTrack::create_video_track(
                    "Shared window",
                    RtcVideoSource::Native(src.clone()),
                );
                match lk
                    .local_participant()
                    .publish_track(
                        LocalTrack::Video(video),
                        TrackPublishOptions {
                            source: TrackSource::Screenshare,
                            simulcast: false,
                            ..Default::default()
                        },
                    )
                    .await
                {
                    Ok(p) => {
                        self.published = Some(p.sid());
                        let cryptor = lk
                            .e2ee_manager()
                            .frame_cryptors()
                            .remove(&(lk.local_participant().identity(), p.sid()));
                        if p.encryption_type() == EncryptionType::Gcm
                            && let Some(fc) = cryptor
                            && fc.enabled()
                        {
                            fc.set_key_index(i32::from(index));
                            if fc.key_index() != i32::from(index) {
                                return Err("Could not encrypt shared window.");
                            }
                            self.frames.spawn(cap.feed(src, mode.clone(), id));
                            Ok("Sharing the selected window")
                        } else {
                            Err("Shared window encryption unavailable.")
                        }
                    }
                    Err(_) => Err("Could not publish the selected window."),
                }
            }
            Update::Chosen(Err(e)) | Update::Ended(e) => Err(e),
        };
        if outcome.is_err() {
            self.stop(lk).await?;
        }
        let _ = events
            .send(
                json!({"command":"window_share","data":{"room":room,"request":id.to_string(),
            "live":outcome.is_ok(),"label":outcome.unwrap_or_else(|e|e)}}),
            )
            .await;
        Ok(())
    }
}
