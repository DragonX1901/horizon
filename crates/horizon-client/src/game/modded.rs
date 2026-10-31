//! Modded single player: a world whose mods run in Java, on a NeoForge 1.21.1
//! server that `horizon-neoforge-host` starts for this window alone
//! (crates/horizon-neoforge-bridge, docs/neoforge-bridge.md). The window joins it
//! like a server, and the world keeps single player's rules: the pause screen
//! pauses and saves it, and nobody else joins until it is opened to LAN.
//!
//! A world is modded when its folder holds `horizon-neoforge.txt`: its first
//! line names the NeoForge server installation (with its mods) to run it on,
//! and an optional `java=PATH` line the Java to run that server with (mods
//! such as Cobblemon need the Java 21 that 1.21.1 ships with).
use super::*;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, channel};

/// The file that makes a world modded.
pub(super) const MARKER: &str = "horizon-neoforge.txt";

/// How a modded world runs: its NeoForge installation and the Java for it.
pub(super) struct Marker {
    pub server: PathBuf,
    pub java: Option<PathBuf>,
}

/// The world's marker, when it is modded.
pub(super) fn marker(world: &Path) -> Option<Marker> {
    let text = std::fs::read_to_string(world.join(MARKER)).ok()?;
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    let resolve = |p: &str| {
        let p = PathBuf::from(p);
        if p.is_absolute() { p } else { world.join(p) }
    };
    let server = resolve(lines.next()?);
    let java = lines.find_map(|l| l.strip_prefix("java=")).map(resolve);
    Some(Marker { server, java })
}

pub(super) struct ModdedHost {
    child: Child,
    stdin: ChildStdin,
    /// The loopback port, once the server is up (taken by the join).
    ready: Option<Receiver<u16>>,
    /// The LAN port, after `lan`.
    lan_port: Receiver<u16>,
    paused: bool,
    lan: Option<u16>,
}

/// A modded world's server started with the game (`--world`), booting while the game loads:
/// the world's folder and its host.
static EARLY: Mutex<Option<(PathBuf, ModdedHost)>> = Mutex::new(None);

/// Start the server of the world the game was asked to open (`--world`), when it is modded, so
/// that NeoForge boots (about 40 s with a big pack) while the game loads its own resources.
pub(super) fn start_early(options: &Options) {
    let Some(folder) = &options.world else { return };
    let Ok(dir) = std::env::current_dir().map(|d| d.join("saves").join(folder)) else {
        return;
    };
    let Some(marker) = marker(&dir) else { return };
    let player = options.player_name();
    match ModdedHost::spawn(&marker, &dir, folder, &player) {
        Ok(host) => *EARLY.lock().unwrap_or_else(|e| e.into_inner()) = Some((dir, host)),
        Err(e) => log::warn!("Heisen: the modded world's server did not start early: {e}"),
    }
}

/// The host started early for `world`, if any.
pub(super) fn take_early(world: &Path) -> Option<ModdedHost> {
    let mut early = EARLY.lock().unwrap_or_else(|e| e.into_inner());
    match early.take() {
        Some((dir, host)) if dir == world => Some(host),
        other => {
            *early = other;
            None
        }
    }
}

impl ModdedHost {
    /// Start the world's server without waiting for it: [`Self::join_address`] is where the
    /// game joins, once it is up.
    pub fn spawn(marker: &Marker, world: &Path, name: &str, owner: &str) -> Result<Self> {
        let exe = std::env::current_exe()?.with_file_name(format!(
            "horizon-neoforge-host{}",
            std::env::consts::EXE_SUFFIX
        ));
        let mut child = Command::new(&exe)
            .arg("--server-dir")
            .arg(&marker.server)
            .arg("--world")
            .arg(world)
            .arg("--name")
            .arg(name)
            .arg("--owner")
            .arg(owner)
            .args(
                marker
                    .java
                    .iter()
                    .flat_map(|j| [std::ffi::OsStr::new("--java"), j.as_os_str()]),
            )
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Could not start {}: {e}", exe.display()))?;
        let stdin = child.stdin.take().ok_or("no host stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or("no host stdout")?);
        let (ready_tx, ready) = channel();
        let (lan_tx, lan_port) = channel();
        std::thread::Builder::new()
            .name("modded-host".into())
            .spawn(move || {
                if let Some(port) = read_port(&mut stdout, "HORIZON_HOST_READY") {
                    let _ = ready_tx.send(port);
                }
                while let Some(port) = read_port(&mut stdout, "HORIZON_HOST_LAN") {
                    let _ = lan_tx.send(port);
                }
            })?;
        Ok(Self {
            child,
            stdin,
            ready: Some(ready),
            lan_port,
            paused: false,
            lan: None,
        })
    }

    /// The address the game joins: the network thread waits there for the server to come up
    /// (horizon_client::neoforge::local_host), so the window keeps drawing meanwhile.
    pub fn join_address(&mut self, resources: &Path) -> Result<String> {
        let ready = self
            .ready
            .take()
            .ok_or("the modded world's server was joined already")?;
        // The game's loaded resources, mods' content included: the bridge translates with them.
        self.send(&format!("resources {}", resources.display()));
        Ok(horizon_client::neoforge::local_host(ready))
    }

    fn send(&mut self, command: &str) {
        let _ = writeln!(self.stdin, "{command}");
        let _ = self.stdin.flush();
    }

    /// `IntegratedServer.tickServer`'s pause: the world stops (and saves) while a
    /// pausing screen is open.
    pub fn set_paused(&mut self, paused: bool) {
        if paused != self.paused {
            self.paused = paused;
            self.send(if paused { "pause" } else { "resume" });
        }
    }

    /// Open to LAN: the port guests join on.
    pub fn publish(&mut self) -> Result<u16> {
        if let Some(port) = self.lan {
            return Ok(port);
        }
        self.send("lan");
        let port = self
            .lan_port
            .recv_timeout(std::time::Duration::from_secs(60))
            .map_err(|_| "the modded world's server stopped")?;
        self.lan = Some(port);
        Ok(port)
    }

    pub fn published(&self) -> Option<u16> {
        self.lan
    }
}

/// The port on the first `<prefix> <port>` line.
fn read_port(stdout: &mut impl BufRead, prefix: &str) -> Option<u16> {
    let mut line = String::new();
    while stdout.read_line(&mut line).ok()? > 0 {
        if let Some(port) = line.trim().strip_prefix(prefix) {
            return port.trim().parse().ok();
        }
        line.clear();
    }
    None
}

impl Drop for ModdedHost {
    /// Leaving the world saves it and stops its server.
    fn drop(&mut self) {
        self.send("stop");
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_ready_port() {
        let mut out = "[server] starting\nHORIZON_HOST_READY 45999\n".as_bytes();
        assert_eq!(read_port(&mut out, "HORIZON_HOST_READY"), Some(45999));
        assert_eq!(
            read_port(&mut "nothing\n".as_bytes(), "HORIZON_HOST_READY"),
            None
        );
    }

    #[test]
    fn marker_names_the_server() {
        let dir = std::env::temp_dir().join(format!("horizon-modded-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(marker(&dir).is_none());
        std::fs::write(
            dir.join(MARKER),
            "/opt/neoforge\njava=/opt/jre21/bin/java\n",
        )
        .unwrap();
        let m = marker(&dir).unwrap();
        assert_eq!(m.server, PathBuf::from("/opt/neoforge"));
        assert_eq!(m.java, Some(PathBuf::from("/opt/jre21/bin/java")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
