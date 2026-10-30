//! A bridge that lets the Horizon client join a NeoForge 1.21.1 server.
//!
//! The mods run in real Java on the server; Horizon draws what they send. The
//! bridge listens for the Horizon client (protocol 777), connects to the server
//! as a NeoForge client would (protocol 767), answers NeoForge's network
//! negotiation itself and records what the server syncs (channels, frozen
//! registries, block states) so the client can be given matching content.
//!
//! Status, by connection state ([docs/neoforge-bridge.md](../../../docs/neoforge-bridge.md)):
//! handshake, status and offline login are translated; configuration is
//! negotiated and captured; play is not translated yet, so the client is shown
//! what was captured and disconnected at the end of configuration.
#![forbid(unsafe_code)]
pub mod blocks;
pub mod facts;
pub mod host;
pub mod ids_767;
pub mod neoforge;
pub mod play;
pub mod session;

/// The protocol version of Minecraft 1.21.1, which NeoForge 21.1 runs on.
pub const PROTOCOL_767: i32 = 767;
/// The game version the bridge's server side speaks.
pub const GAME_VERSION_767: &str = "1.21.1";
