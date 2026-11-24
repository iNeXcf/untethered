//! Local CALL-01 media test. No capture/playout hardware or Matrix account.
use anyhow::{Result, bail, ensure};
use futures_util::StreamExt;
use livekit::{
    e2ee::{E2eeOptions, EncryptionType, key_provider::KeyProvider},
    options::TrackPublishOptions,
    prelude::*,
    webrtc::{
        audio_source::native::NativeAudioSource,
        audio_stream::native::NativeAudioStream,
        prelude::{AudioFrame, AudioSourceOptions, RtcAudioSource},
    },
};
use std::{env, time::Duration};

fn options(keys: &[[u8; 32]; 2]) -> Result<RoomOptions> {
    let provider = KeyProvider::new(Default::default());
    for (i, key) in keys.iter().enumerate() {
        ensure!(
            provider.set_key(&format!("probe-{i}").into(), 0, key.to_vec()),
            "could not install test media key"
        );
    }
    let mut opts = RoomOptions::default();
    opts.encryption = Some(E2eeOptions {
        encryption_type: EncryptionType::Gcm,
        key_provider: provider,
    });
    Ok(opts)
}
async fn publish(room: &Room, hz: f64) -> Result<tokio::task::JoinHandle<()>> {
    let src = NativeAudioSource::new(AudioSourceOptions::default(), 48000, 1, 50);
    let track =
        LocalAudioTrack::create_audio_track("test-tone", RtcAudioSource::Native(src.clone()));
    room.local_participant()
        .publish_track(LocalTrack::Audio(track), TrackPublishOptions::default())
        .await?;
    Ok(tokio::spawn(async move {
        let mut index = 0_u64;
        let mut clock = tokio::time::interval(Duration::from_millis(10));
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            clock.tick().await;
            let data = (0..480)
                .map(|_| {
                    let y = (index as f64 * std::f64::consts::TAU * hz / 48000.).sin();
                    index += 1;
                    (y * 9000.) as i16
                })
                .collect::<Vec<_>>();
            if src
                .capture_frame(&AudioFrame {
                    data: data.into(),
                    sample_rate: 48000,
                    num_channels: 1,
                    samples_per_channel: 480,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    }))
}
async fn receive(
    mut events: tokio::sync::mpsc::UnboundedReceiver<RoomEvent>,
    hz: f64,
) -> Result<()> {
    while let Some(event) = events.recv().await {
        if let RoomEvent::TrackSubscribed {
            track: RemoteTrack::Audio(track),
            publication,
            ..
        } = event
        {
            ensure!(
                publication.encryption_type() == EncryptionType::Gcm,
                "received unencrypted publication"
            );
            let mut stream = NativeAudioStream::new(track.rtc_track(), 48000, 1);
            let mut stable = 0;
            while let Some(f) = stream.next().await {
                // Require a sustained tone at the *other* peer's frequency;
                // silence, successful signaling, and our own echo don't pass.
                let crossings = f.data.windows(2).filter(|w| w[0] <= 0 && w[1] > 0).count();
                let estimate = crossings as f64 * 48000. / f.data.len() as f64;
                if (estimate - hz).abs() < 110. && f.data.iter().any(|&v| v.unsigned_abs() > 800) {
                    stable += 1;
                } else {
                    stable = 0;
                }
                if stable >= 20 {
                    return Ok(());
                }
            }
        }
    }
    bail!("no decoded tone")
}
#[tokio::main(worker_threads = 2)]
async fn main() -> Result<()> {
    let mut keys = [[0_u8; 32]; 2];
    for k in &mut keys {
        getrandom::fill(k).map_err(|_| anyhow::anyhow!("OS randomness unavailable"))?;
    }
    let port = env::var("RTC_PROBE_PORT")?.parse::<u16>()?;
    let url = format!("ws://127.0.0.1:{port}");
    tokio::time::timeout(Duration::from_secs(45),async {
        let (a,ae)=Room::connect(&url,&env::var("RTC_TOKEN_A")?,options(&keys)?).await?;
        println!("First peer joined.");
        let (b,be)=Room::connect(&url,&env::var("RTC_TOKEN_B")?,options(&keys)?).await?;
        println!("Second peer joined.");
        let (at,bt)=tokio::try_join!(publish(&a,480.),publish(&b,960.))?;
        println!("Published both audio tracks; checking decoded PCM.");
        let result=tokio::try_join!(receive(ae,960.),receive(be,480.));
        at.abort();bt.abort(); let _=tokio::join!(a.close(),b.close());result?;
        println!("PASS: two native peers decoded each other's distinct tones on encrypted LiveKit audio tracks.");
        Ok::<(),anyhow::Error>(())
    }).await??;
    Ok(())
}
