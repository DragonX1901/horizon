//! Modded single player: a world whose mods run in Java, on a NeoForge 1.21.1
//! server that `horizon-neoforge-host` starts for this window alone
//! (crates/horizon-neoforge-bridge, docs/neoforge-bridge.md). The window joins it
//! like a server, and the world keeps single player's rules: the pause screen
//! pauses and saves it, and nobody else joins until it is opened to LAN.
//!
//! A world is modded when its folder holds `horizon-neoforge.txt`, naming the
//! NeoForge server installation (with its mods) to run it on.
use super::*;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// The file that makes a world modded.
pub(super) const MARKER: &str = "horizon-neoforge.txt";

/// The NeoForge installation a world runs on, when it is modded.
pub(super) fn server_dir(world: &Path) -> Option<PathBuf> {
    let text = std::fs::read_to_string(world.join(MARKER)).ok()?;
    let dir = PathBuf::from(text.trim());
    Some(if dir.is_absolute() {
        dir
    } else {
        world.join(dir)
    })
}

pub(super) struct ModdedHost {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    paused: bool,
    lan: Option<u16>,
}

impl ModdedHost {
    /// Start the world's server and wait for it: the loopback port to join.
    pub fn start(
        server: &Path,
        world: &Path,
        resources: &Path,
        name: &str,
        owner: &str,
    ) -> Result<(Self, u16)> {
        let exe = std::env::current_exe()?.with_file_name(format!(
            "horizon-neoforge-host{}",
            std::env::consts::EXE_SUFFIX
        ));
        let mut child = Command::new(&exe)
            .arg("--server-dir")
            .arg(server)
            .arg("--world")
            .arg(world)
            .arg("--resources")
            .arg(resources)
            .arg("--name")
            .arg(name)
            .arg("--owner")
            .arg(owner)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Could not start {}: {e}", exe.display()))?;
        let stdin = child.stdin.take().ok_or("no host stdin")?;
        let mut stdout = BufReader::new(child.stdout.take().ok_or("no host stdout")?);
        let port = read_port(&mut stdout, "HORIZON_HOST_READY")
            .ok_or("The modded world's server stopped while starting; see the log")?;
        let host = Self {
            child,
            stdin,
            stdout,
            paused: false,
            lan: None,
        };
        Ok((host, port))
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
        let port = read_port(&mut self.stdout, "HORIZON_HOST_LAN")
            .ok_or("the modded world's server stopped")?;
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
        assert_eq!(server_dir(&dir), None);
        std::fs::write(dir.join(MARKER), "/opt/neoforge\n").unwrap();
        assert_eq!(server_dir(&dir), Some(PathBuf::from("/opt/neoforge")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
