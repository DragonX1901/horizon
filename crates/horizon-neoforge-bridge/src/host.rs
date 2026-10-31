//! A modded single-player world: a NeoForge server only this machine can reach,
//! and a bridge only its owner joins, kept to single player's rules.
//!
//! - The server binds 127.0.0.1 on a free port and never takes players itself.
//! - The bridge's owner listener (loopback) lets one player in. A second login
//!   waits for `lan`.
//! - The game drives it over stdin, one command per line: `pause` (Esc: freeze
//!   the server's ticks and save, as the integrated server does on pause),
//!   `resume`, `lan` (Open to LAN: a listener on every interface and the LAN
//!   broadcast), `stop` (save and quit). Closing stdin is `stop`.
//! - It prints `HORIZON_HOST_READY <port>` once the owner can join, and
//!   `HORIZON_HOST_LAN <port>` once LAN is open.
use crate::session::{Options, handle};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpListener, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub struct HostOptions {
    /// An installed NeoForge 21.1 server (its `libraries/` and the mods in `mods/`).
    pub server_dir: PathBuf,
    /// The world folder.
    pub world: PathBuf,
    /// The client's resources, for play translation. None: the game sends them on stdin
    /// (`resources <path>`) once it has loaded them, which lets the server boot meanwhile.
    pub resources: Option<PathBuf>,
    pub java: PathBuf,
    /// The world's name, for the LAN announcement.
    pub name: String,
    /// The owner's player name: made an operator, as a single-player world with
    /// commands allowed lets its player use them.
    pub owner: Option<String>,
}

fn free_port(ip: Ipv4Addr) -> std::io::Result<u16> {
    Ok(TcpListener::bind(SocketAddrV4::new(ip, 0))?
        .local_addr()?
        .port())
}

/// `server.properties` for a world only this machine reaches. Everything else in
/// an existing file is kept.
fn write_properties(server_dir: &Path, world: &Path, port: u16) -> std::io::Result<()> {
    let path = server_dir.join("server.properties");
    let forced = [
        ("server-ip", "127.0.0.1".to_string()),
        ("server-port", port.to_string()),
        ("online-mode", "false".into()),
        ("enable-rcon", "false".into()),
        ("enable-query", "false".into()),
        (
            "level-name",
            world.display().to_string().replace('\\', "\\\\"),
        ),
    ];
    let existing = std::fs::read_to_string(&path).unwrap_or_default();
    let mut lines: Vec<String> = existing
        .lines()
        .filter(|l| !forced.iter().any(|(k, _)| l.starts_with(&format!("{k}="))))
        .map(String::from)
        .collect();
    lines.extend(forced.iter().map(|(k, v)| format!("{k}={v}")));
    std::fs::write(&path, lines.join("\n") + "\n")?;
    // Single player runs on the player's own copy of the game, whose EULA they
    // accepted to play it; the integrated server it stands in for asks nothing.
    std::fs::write(server_dir.join("eula.txt"), "eula=true\n")
}

/// The NeoForge version installed in `server_dir`, from its libraries.
fn neoforge_version(server_dir: &Path) -> Option<String> {
    let dir = server_dir.join("libraries/net/neoforged/neoforge");
    let mut versions: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|v| v.starts_with("21.1."))
        .collect();
    versions.sort_by_key(|v| v.rsplit('.').next().and_then(|n| n.parse::<u32>().ok()));
    versions.pop()
}

fn start_server(options: &HostOptions) -> std::io::Result<(Child, u16)> {
    let port = free_port(Ipv4Addr::LOCALHOST)?;
    write_properties(&options.server_dir, &options.world, port)?;
    let version = neoforge_version(&options.server_dir)
        .ok_or_else(|| std::io::Error::other("no NeoForge 21.1 installed in the server folder"))?;
    let args = format!("@libraries/net/neoforged/neoforge/{version}/unix_args.txt");
    #[cfg(windows)]
    let args = format!("@libraries/net/neoforged/neoforge/{version}/win_args.txt");
    let child = Command::new(&options.java)
        .current_dir(&options.server_dir)
        .arg("-Xmx4G")
        .arg(args)
        .arg("nogui")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    Ok((child, port))
}

/// Wait for the server's "Done (…)!" line, then echo its log to stderr and
/// make `owner` an operator when they join: an offline-mode server resolves a
/// name to its player only while they are online.
fn wait_until_ready(
    child: &mut Child,
    owner: Option<String>,
    stdin: Arc<Mutex<ChildStdin>>,
) -> std::io::Result<()> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("no server stdout"))?;
    let mut lines = BufReader::new(stdout).lines();
    for line in lines.by_ref() {
        let line = line?;
        eprintln!("[server] {line}");
        if line.contains("]: Done (") {
            std::thread::spawn(move || {
                let joined = owner.as_ref().map(|o| format!("]: {o} joined the game"));
                for line in lines.map_while(Result::ok) {
                    eprintln!("[server] {line}");
                    if let (Some(owner), Some(joined)) = (&owner, &joined)
                        && line.contains(joined.as_str())
                    {
                        console(&stdin, &format!("op {owner}"));
                    }
                }
            });
            return Ok(());
        }
    }
    Err(std::io::Error::other(
        "the server stopped before it was ready",
    ))
}

fn console(stdin: &Mutex<ChildStdin>, command: &str) {
    if let Ok(mut s) = stdin.lock() {
        let _ = writeln!(s, "{command}");
        let _ = s.flush();
    }
}

/// Accept on `listener`; the session refuses logins over its `max_players`.
fn accept(listener: TcpListener, bridge: Arc<Options>) {
    for stream in listener.incoming().map_while(Result::ok) {
        let bridge = bridge.clone();
        std::thread::spawn(move || {
            if let Err(e) = handle(stream, &bridge)
                && !e.is_end_of_stream()
            {
                eprintln!("[host] {e}");
            }
        });
    }
}

/// LanServerPinger: announce the world on 224.0.2.60:4445 every 1.5 s.
fn announce(name: String, port: u16, open: Arc<AtomicBool>) {
    let Ok(socket) = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)) else {
        return;
    };
    let ping = format!("[MOTD]{name}[/MOTD][AD]{port}[/AD]");
    while open.load(Ordering::Relaxed) {
        let _ = socket.send_to(ping.as_bytes(), (Ipv4Addr::new(224, 0, 2, 60), 4445));
        std::thread::sleep(Duration::from_millis(1500));
    }
}

/// Run the world until `stop` or stdin closes.
pub fn run(options: HostOptions) -> std::io::Result<()> {
    let (mut child, server_port) = start_server(&options)?;
    let server_stdin = Arc::new(Mutex::new(
        child
            .stdin
            .take()
            .ok_or_else(|| std::io::Error::other("no server stdin"))?,
    ));
    wait_until_ready(&mut child, options.owner.clone(), server_stdin.clone())?;
    let mut input = std::io::stdin().lock().lines();
    let resources = match options.resources.clone() {
        Some(r) => r,
        None => loop {
            match input.next().transpose()?.as_deref().map(str::trim) {
                Some(line) if line.starts_with("resources ") => break PathBuf::from(&line[10..]),
                Some("stop") | None => {
                    console(&server_stdin, "stop");
                    child.wait()?;
                    return Ok(());
                }
                Some(_) => {}
            }
        },
    };
    // The owner's listener lets one player in, whoever else is playing: a second
    // login there is refused until LAN is open, and LAN players join on their own.
    let owner_bridge = Options {
        server_host: "127.0.0.1".into(),
        server_port,
        dump_dir: None,
        resources: Some(resources),
        players: Arc::new(AtomicUsize::new(0)),
        authenticate: None,
        max_players: Some(1),
    };
    let lan_bridge = Arc::new(Options {
        players: Arc::new(AtomicUsize::new(0)),
        authenticate: None,
        max_players: None,
        ..owner_bridge.clone()
    });
    let owner = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let owner_port = owner.local_addr()?.port();
    {
        let bridge = Arc::new(owner_bridge);
        std::thread::spawn(move || accept(owner, bridge));
    }
    println!("HORIZON_HOST_READY {owner_port}");
    let lan_open = Arc::new(AtomicBool::new(false));
    let mut paused = false;
    for line in input {
        match line?.trim() {
            "pause" if !paused => {
                paused = true;
                console(&server_stdin, "tick freeze");
                console(&server_stdin, "save-all flush");
            }
            "resume" if paused => {
                paused = false;
                console(&server_stdin, "tick unfreeze");
            }
            "lan" if !lan_open.load(Ordering::Relaxed) => {
                let listener = TcpListener::bind((Ipv4Addr::UNSPECIFIED, 0))?;
                let port = listener.local_addr()?.port();
                lan_open.store(true, Ordering::Relaxed);
                let bridge = lan_bridge.clone();
                std::thread::spawn(move || accept(listener, bridge));
                let (name, open) = (options.name.clone(), lan_open.clone());
                std::thread::spawn(move || announce(name, port, open));
                println!("HORIZON_HOST_LAN {port}");
            }
            "stop" => break,
            _ => {}
        }
    }
    lan_open.store(false, Ordering::Relaxed);
    if paused {
        console(&server_stdin, "tick unfreeze");
    }
    console(&server_stdin, "stop");
    child.wait()?;
    Ok(())
}
