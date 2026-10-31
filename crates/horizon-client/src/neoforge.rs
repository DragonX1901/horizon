//! Multiplayer on NeoForge 1.21.1 servers: a server that answers the status ping with
//! 1.21.1's protocol (767) is joined through the NeoForge bridge, run inside the game on
//! a loopback port (crates/horizon-neoforge-bridge, docs/neoforge-bridge.md). The bridge
//! logs in as the player, online-mode servers included, and translates the rest.
//!
//! The mods' blocks, items and mobs show when their content mods are installed
//! (`tools/neoforge_bridge/setup_world.py --join`), and the server needs the bridge's
//! companion mod.
use horizon_neoforge_bridge::session::{Authenticator, Options, serve};
use std::collections::HashMap;
use std::net::{Ipv4Addr, TcpListener};
use std::path::Path;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The bridge already started for each server address: its loopback port.
static BRIDGES: Mutex<Option<HashMap<String, u16>>> = Mutex::new(None);

/// The address prefix of a modded single-player world whose server is still starting.
const LOCAL_HOST: &str = "heisen-modded:";

/// Modded worlds' servers being started: the port each one sends once the owner can join.
static STARTING: Mutex<Option<HashMap<String, std::sync::mpsc::Receiver<u16>>>> = Mutex::new(None);

/// An address to join a modded world's server at, once `ready` names its port.
pub fn local_host(ready: std::sync::mpsc::Receiver<u16>) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let address = format!(
        "{LOCAL_HOST}{}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    );
    STARTING
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(address.clone(), ready);
    address
}

/// For a [`local_host`] address: wait for its server (the network thread, with `status`
/// shown meanwhile), and the loopback address it listens on.
pub fn wait_local(address: &str, status: impl Fn(&str)) -> Option<Result<String, String>> {
    if !address.starts_with(LOCAL_HOST) {
        return None;
    }
    let ready = STARTING
        .lock()
        .unwrap()
        .as_mut()
        .and_then(|s| s.remove(address));
    let Some(ready) = ready else {
        return Some(Err("The modded world's server is gone".into()));
    };
    status("Starting the world's mods...");
    Some(
        ready
            .recv_timeout(Duration::from_secs(15 * 60))
            .map(|port| format!("127.0.0.1:{port}"))
            .map_err(|_| "The modded world's server stopped while starting; see the log".into()),
    )
}

/// Where to connect for `address`: a loopback bridge when it is a NeoForge 1.21.1
/// server, else None (join it directly).
pub fn route(address: &str, resources: &Path) -> Result<Option<String>, String> {
    let Some((host, port)) = horizon_protocol::servers::parse_address(address) else {
        return Ok(None);
    };
    let mut bridges = BRIDGES.lock().unwrap();
    let bridges = bridges.get_or_insert_with(HashMap::new);
    if let Some(port) = bridges.get(address) {
        return Ok(Some(format!("127.0.0.1:{port}")));
    }
    let Ok((status, _)) = crate::net::ping(address, Duration::from_secs(5)) else {
        return Ok(None);
    };
    if status["version"]["protocol"].as_i64() != Some(horizon_neoforge_bridge::PROTOCOL_767.into())
    {
        return Ok(None);
    }
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .map_err(|e| format!("Could not start the NeoForge bridge: {e}"))?;
    let local = listener.local_addr().map_err(|e| e.to_string())?.port();
    // The account signed in when the player joins, not when the bridge started.
    let authenticate: Authenticator = Arc::new(|hash: &str| {
        let account = crate::auth::current().ok_or(crate::net::NOT_SIGNED_IN)?;
        crate::auth::join_server(&account, hash).map_err(|e| match e.reason() {
            Ok(key) => key.to_owned(),
            Err(message) => message.to_owned(),
        })
    });
    let options = Arc::new(Options {
        server_host: host,
        server_port: port,
        dump_dir: None,
        resources: Some(resources.to_owned()),
        players: Arc::new(AtomicUsize::new(0)),
        // Only this game, one session at a time: the port is loopback, and it signs in
        // as the player.
        max_players: Some(1),
        authenticate: Some(authenticate),
    });
    std::thread::Builder::new()
        .name("neoforge-bridge".into())
        .spawn(move || {
            if let Err(e) = serve(listener, options) {
                log::warn!("Heisen: the NeoForge bridge stopped: {e}");
            }
        })
        .map_err(|e| e.to_string())?;
    log::info!(
        "Heisen: {} is a NeoForge 1.21.1 server; joining through the bridge",
        crate::net::shown_address(address)
    );
    bridges.insert(address.to_owned(), local);
    Ok(Some(format!("127.0.0.1:{local}")))
}
