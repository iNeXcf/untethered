//! Linux call-local PulseAudio routes. A device name, not its transient index
//! or display label, is passed to pacat. None leaves the session default intact.
use serde_json::Value;
use std::{collections::BTreeMap, fmt, time::Duration};
use tokio::{process::Command, time::timeout};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Port {
    Mic,
    Speaker,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Route {
    pub name: Option<String>,
    // Selecting the same endpoint again retries it after a device failure.
    pub revision: u64,
}

#[derive(Clone, PartialEq, Eq)]
pub struct Choice {
    pub name: Option<String>,
    label: String,
}
impl fmt::Display for Choice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}
impl Choice {
    fn system() -> Self {
        Self {
            name: None,
            label: "System default".into(),
        }
    }
}

#[derive(Clone)]
pub struct Catalog {
    pub mics: Vec<Choice>,
    pub speakers: Vec<Choice>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            mics: vec![Choice::system()],
            speakers: vec![Choice::system()],
        }
    }
}
impl Catalog {
    pub fn list(&self, port: Port) -> &[Choice] {
        match port {
            Port::Mic => &self.mics,
            Port::Speaker => &self.speakers,
        }
    }
    pub fn selected(&self, port: Port, route: &Route) -> Choice {
        self.list(port)
            .iter()
            .find(|c| c.name == route.name)
            .cloned()
            .unwrap_or_else(|| Choice {
                name: route.name.clone(),
                label: format!(
                    "Unavailable: {}",
                    route.name.as_deref().unwrap_or("default")
                ),
            })
    }
    pub async fn read() -> Result<Self, &'static str> {
        let (mics, speakers) = tokio::try_join!(devices("sources"), devices("sinks"))?;
        Ok(Self { mics, speakers })
    }
}
async fn devices(kind: &str) -> Result<Vec<Choice>, &'static str> {
    let output = timeout(
        Duration::from_secs(4),
        Command::new("pactl")
            .args(["--format=json", "list", kind])
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| "Audio device query timed out.")?
    .map_err(|_| "Cannot list audio devices. Is pactl installed?")?;
    if !output.status.success() {
        return Err("Audio server unavailable. Refresh to try again.");
    }
    let rows: Vec<Value> =
        serde_json::from_slice(&output.stdout).map_err(|_| "Invalid audio device list.")?;
    let mut choices: Vec<_> = rows
        .iter()
        .filter_map(|v| {
            let name = v["name"].as_str().filter(|s| !s.is_empty())?;
            let label = v["description"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(name);
            Some(Choice {
                name: Some(name.to_owned()),
                label: label.to_owned(),
            })
        })
        .collect();
    let mut counts = BTreeMap::new();
    for c in &choices {
        *counts.entry(c.label.clone()).or_insert(0) += 1;
    }
    for c in &mut choices {
        if counts[&c.label] > 1 {
            c.label = format!("{} ({})", c.label, c.name.as_deref().unwrap());
        }
    }
    choices.sort_by(|a, b| a.label.cmp(&b.label));
    choices.insert(0, Choice::system());
    Ok(choices)
}
