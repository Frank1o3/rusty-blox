use serde_json::{Value, json};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

pub(crate) struct GameMode {
    connection: Option<zbus::blocking::Connection>,
    registered: bool,
}

impl GameMode {
    pub(crate) fn register(enabled: bool) -> Self {
        let mut result = Self {
            connection: None,
            registered: false,
        };
        if !enabled {
            return result;
        }
        let outcome = (|| -> Result<i32, Box<dyn std::error::Error>> {
            let conn = zbus::blocking::Connection::session()?;
            let reply = conn.call_method(
                Some("com.feralinteractive.GameMode"),
                "/com/feralinteractive/GameMode",
                Some("com.feralinteractive.GameMode"),
                "RegisterGame",
                &(std::process::id() as i32,),
            )?;
            let code = reply.body().deserialize::<i32>()?;
            result.connection = Some(conn);
            Ok(code)
        })();
        match outcome {
            Ok(0) => {
                result.registered = true;
                eprintln!("rusty-blox: registered with Feral GameMode");
            }
            Ok(code) => eprintln!("rusty-blox: GameMode declined registration ({code})"),
            Err(error) => eprintln!("rusty-blox: GameMode unavailable, continuing: {error}"),
        }
        result
    }
}

impl Drop for GameMode {
    fn drop(&mut self) {
        if !self.registered {
            return;
        }
        if let Some(conn) = &self.connection {
            let call = conn.call_method(
                Some("com.feralinteractive.GameMode"),
                "/com/feralinteractive/GameMode",
                Some("com.feralinteractive.GameMode"),
                "UnregisterGame",
                &(std::process::id() as i32,),
            );
            match call.and_then(|reply| reply.body().deserialize::<i32>().map_err(Into::into)) {
                Ok(0) => eprintln!("rusty-blox: unregistered from Feral GameMode"),
                Ok(code) => eprintln!("rusty-blox: GameMode unregister returned {code}"),
                Err(error) => eprintln!("rusty-blox: GameMode unregister failed: {error}"),
            }
        }
    }
}

pub(crate) struct DiscordPresence(Option<UnixStream>);

impl DiscordPresence {
    pub(crate) fn connect(enabled: bool, app_id: &str) -> Self {
        if !enabled {
            return Self(None);
        }
        if app_id.trim().is_empty() {
            eprintln!(
                "rusty-blox: Discord presence enabled but no Discord Application ID is configured"
            );
            return Self(None);
        }
        match connect_presence(app_id.trim()) {
            Ok(stream) => {
                eprintln!("rusty-blox: Discord Rich Presence connected");
                Self(Some(stream))
            }
            Err(error) => {
                eprintln!("rusty-blox: Discord presence unavailable, continuing: {error}");
                Self(None)
            }
        }
    }
}

impl Drop for DiscordPresence {
    fn drop(&mut self) {
        if let Some(stream) = self.0.as_mut() {
            let _ = write_frame(
                stream,
                1,
                &json!({"cmd":"SET_ACTIVITY", "args":{"pid":std::process::id(), "activity":null}, "nonce":"rusty-blox-clear"}),
            );
        }
    }
}

fn connect_presence(client_id: &str) -> Result<UnixStream, String> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .ok_or("XDG_RUNTIME_DIR is unset")?;
    let mut candidates = Vec::new();
    for slot in 0..10 {
        candidates.push(runtime.join(format!("discord-ipc-{slot}")));
    }
    for slot in 0..10 {
        candidates.push(
            runtime
                .join("app/com.discordapp.Discord")
                .join(format!("discord-ipc-{slot}")),
        );
    }
    let mut last = "Discord IPC socket not found".to_string();
    for path in candidates {
        let Ok(mut stream) = UnixStream::connect(path) else {
            continue;
        };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
        if write_frame(&mut stream, 0, &json!({"v":1,"client_id":client_id})).is_err() {
            continue;
        }
        if read_frame(&mut stream).is_err() {
            last = "Discord IPC handshake failed".into();
            continue;
        }
        if write_frame(&mut stream, 1, &json!({"cmd":"SET_ACTIVITY","args":{"pid":std::process::id(),"activity":{"type":0,"details":"Playing Roblox","assets":{"large_text":"Roblox"}}},"nonce":"rusty-blox-presence"})).is_err() {
            last = "Discord rejected the activity frame".into(); continue;
        }
        let _ = read_frame(&mut stream);
        return Ok(stream);
    }
    Err(last)
}

fn write_frame(stream: &mut UnixStream, opcode: u32, body: &Value) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(body).map_err(std::io::Error::other)?;
    stream.write_all(&opcode.to_le_bytes())?;
    stream.write_all(&(bytes.len() as u32).to_le_bytes())?;
    stream.write_all(&bytes)
}

fn read_frame(stream: &mut UnixStream) -> std::io::Result<(u32, Value)> {
    let mut header = [0; 8];
    stream.read_exact(&mut header)?;
    let size = u32::from_le_bytes(header[4..8].try_into().unwrap()) as usize;
    if size > 64 * 1024 {
        return Err(std::io::Error::other("oversized Discord IPC frame"));
    }
    let mut bytes = vec![0; size];
    stream.read_exact(&mut bytes)?;
    Ok((
        u32::from_le_bytes(header[..4].try_into().unwrap()),
        serde_json::from_slice(&bytes).map_err(std::io::Error::other)?,
    ))
}
