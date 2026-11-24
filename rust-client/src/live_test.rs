//! Opt-in acceptance test against a disposable local homeserver.
use super::*;
use async_channel::Receiver;
use std::{env, time::Duration};

async fn wait(app: &mut App, rx: &Receiver<Value>, label: &str, until: impl Fn(&App) -> bool) {
    tokio::time::timeout(Duration::from_secs(45), async {
        while !until(app) {
            let f = rx.recv().await.expect("backend exited");
            assert_ne!(
                f["command"], "error",
                "gomuks request failed during {label}"
            );
            app.frame(f);
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out during {label}"));
}
fn session() -> (
    App,
    Receiver<Value>,
    tokio::task::JoinHandle<Result<(), String>>,
) {
    let (tx, cmd) = async_channel::bounded(64);
    let (output, rx) = async_channel::bounded(64);
    (App::new(tx), rx, tokio::spawn(backend::run(cmd, output)))
}
#[tokio::test]
#[ignore = "requires a disposable loopback homeserver and a new, separate gomuks profile"]
async fn encrypted_round_trip_and_restart() {
    let fixture: Value =
        serde_json::from_slice(&std::fs::read(env::var("UNTETHERED_QA_FIXTURE").unwrap()).unwrap())
            .unwrap();
    let server = fixture["server"].as_str().unwrap();
    assert!(url::Url::parse(server).unwrap().host_str() == Some("127.0.0.1"));
    assert!(
        !std::path::Path::new(&env::var_os("UNTETHERED_PROFILE").unwrap()).exists(),
        "Use a fresh profile."
    );
    let id = fixture["room"].as_str().unwrap();
    let body = format!("Rust client round trip {}", uuid::Uuid::new_v4());
    let (mut app, rx, engine) = session();
    wait(&mut app, &rx, "startup", |a| a.connected).await;
    assert!(!app.logged_in);
    app.server = server.into();
    app.username = rooms::string(&fixture["username"]);
    app.password = rooms::string(&fixture["password"]);
    let _ = app.update(Msg::Login);
    wait(&mut app, &rx, "login", |a| a.logged_in).await;
    // Exercise the error path before retrying. The failed request also
    // dispatches client_state; it must not make the feedback disappear.
    app.recovery = "not a recovery key".into();
    let _ = app.update(Msg::Verify);
    tokio::time::timeout(Duration::from_secs(25), async {
        while app.verifying() {
            app.frame(rx.recv().await.unwrap());
        }
    })
    .await
    .expect("failed verification returned");
    assert!(app.verification_feedback().contains("invalid"));
    app.recovery = rooms::string(&fixture["key"]).replace(' ', "\u{202f}");
    let _ = app.update(Msg::Verify);
    wait(&mut app, &rx, "verified room sync", |a| {
        a.verified && a.live.list.contains_key(id)
    })
    .await;
    app.open(id.into());
    assert!(app.live.list[id].encrypted);
    app.editor = text_editor::Content::with_text(&body);
    let _ = app.update(Msg::Send);
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let f = rx.recv().await.unwrap();
            let done = f["command"] == "send_complete" && f["data"]["event"]["room_id"] == id;
            if done {
                assert!(f["data"]["error"].is_null());
            }
            app.frame(f);
            if done {
                break;
            }
        }
    })
    .await
    .expect("encrypted send completion");
    app.commands.close();
    engine.await.unwrap().unwrap();
    let (mut app, rx, engine) = session();
    wait(&mut app, &rx, "restore session and decrypt history", |a| {
        a.verified
            && a.live
                .list
                .get(id)
                .is_some_and(|r| r.history.values().any(|p| p.body == body))
    })
    .await;
    app.commands.close();
    engine.await.unwrap().unwrap();
}
