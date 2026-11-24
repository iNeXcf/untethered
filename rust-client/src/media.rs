use serde_json::Value;
use std::collections::BTreeMap;

// The browser worker buffers an entire file. Enforce the same initial limit on
// both clients; apply a smaller limit when advertised by the homeserver.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_FILES: usize = 10;

#[derive(Clone)]
pub struct LocalFile {
    pub name: String,
    pub size: u64,
    #[cfg(not(target_arch = "wasm32"))]
    pub path: std::path::PathBuf,
    #[cfg(target_arch = "wasm32")]
    pub file: web_sys::File,
}
#[cfg(not(target_arch = "wasm32"))]
impl LocalFile {
    pub fn from_path(path: std::path::PathBuf) -> Option<Self> {
        let meta = path.metadata().ok()?;
        if !meta.is_file() {
            return None;
        }
        Some(Self {
            name: path.file_name()?.to_string_lossy().into_owned(),
            size: meta.len(),
            path,
        })
    }
}

#[derive(Default)]
pub struct Outbox {
    next: u64,
    pub rooms: BTreeMap<String, Vec<Attachment>>,
}
pub struct Attachment {
    pub id: u64,
    pub source: LocalFile,
    pub descriptor: Option<Value>,
    pub txn: Option<String>,
    pub busy: bool,
    pub error: Option<String>,
}
impl Outbox {
    pub fn stage(&mut self, room: &str, files: Vec<LocalFile>, limit: u64) -> bool {
        let queue = self.rooms.entry(room.to_owned()).or_default();
        let mut all = true;
        for file in files {
            if file.size > limit || queue.len() >= MAX_FILES {
                all = false;
                continue;
            }
            self.next += 1;
            queue.push(Attachment {
                id: self.next,
                source: file,
                descriptor: None,
                txn: None,
                busy: false,
                error: None,
            });
        }
        all
    }
    pub fn get_mut(&mut self, room: &str, id: u64) -> Option<&mut Attachment> {
        self.rooms.get_mut(room)?.iter_mut().find(|a| a.id == id)
    }
    pub fn remove(&mut self, room: &str, id: u64) {
        if let Some(q) = self.rooms.get_mut(room) {
            q.retain(|a| a.id != id);
            if q.is_empty() {
                self.rooms.remove(room);
            }
        }
    }
}
pub fn size(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1 << 20) as f64)
    } else if n >= 1024 {
        format!("{:.1} KiB", n as f64 / 1024.)
    } else {
        format!("{n} B")
    }
}

#[derive(Clone)]
pub struct Media {
    pub name: String,
    pub size: Option<u64>,
    pub mxc: String,
    pub encrypted: bool,
}
impl Media {
    pub fn parse(content: &Value) -> Option<Self> {
        if !matches!(
            content["msgtype"].as_str(),
            Some("m.file" | "m.image" | "m.audio" | "m.video")
        ) {
            return None;
        }
        let encrypted = content["file"].is_object();
        let mxc = if encrypted {
            &content["file"]["url"]
        } else {
            &content["url"]
        }
        .as_str()?;
        media_path(mxc)?;
        // Never use a sender-supplied path as a save destination.
        let name = content["filename"]
            .as_str()
            .or(content["body"].as_str())
            .unwrap_or("attachment")
            .rsplit(['/', '\\'])
            .next()
            .filter(|s| !s.is_empty() && *s != "." && *s != "..")
            .unwrap_or("attachment")
            .chars()
            .filter(|c| !c.is_control())
            .take(200)
            .collect::<String>();
        Some(Self {
            name,
            size: content["info"]["size"].as_u64(),
            mxc: mxc.into(),
            encrypted,
        })
    }
}
pub fn media_path(mxc: &str) -> Option<String> {
    let (server, id) = mxc.strip_prefix("mxc://")?.split_once('/')?;
    if server.is_empty()
        || matches!(server, "." | "..")
        || id.is_empty()
        || !server
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-:[]".contains(&b))
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    {
        return None;
    }
    Some(format!("/_gomuks/media/{server}/{id}"))
}

#[cfg(target_os = "linux")]
pub fn drop_paths(uris: &str) -> Vec<std::path::PathBuf> {
    uris.lines()
        .filter_map(|s| {
            if s.starts_with('#') {
                return None;
            }
            let u = url::Url::parse(s).ok()?;
            if u.scheme() != "file"
                || u.host_str().is_some_and(|s| s != "localhost")
                || u.query().is_some()
                || u.fragment().is_some()
            {
                return None;
            }
            u.to_file_path().ok()
        })
        .take(MAX_FILES + 1)
        .collect()
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    #[test]
    fn uri_lists_handle_multiple_files_and_reject_network_paths() {
        let u = "# ignored\r\nfile:///tmp/a%20b.txt\r\nhttps://example.org/file\nfile://remote/share\nfile:///tmp/c.pdf\r\n";
        assert_eq!(
            super::drop_paths(u),
            [
                std::path::PathBuf::from("/tmp/a b.txt"),
                "/tmp/c.pdf".into()
            ]
        );
    }
}
