//! `horizon-neoforge-bridge --server HOST[:PORT] [--listen ADDR] [--dump DIR]`:
//! Horizon players join `ADDR` (default 127.0.0.1:25566) and reach the NeoForge
//! 1.21.1 server at HOST.
use horizon_neoforge_bridge::session::{Options, serve};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

const USAGE: &str =
    "usage: horizon-neoforge-bridge --server HOST[:PORT] [--listen ADDR] [--dump DIR]";

fn main() {
    let mut server = None;
    let mut listen = "127.0.0.1:25566".to_string();
    let mut dump_dir = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next().unwrap_or_else(|| {
                eprintln!("{arg} needs a value\n{USAGE}");
                std::process::exit(2)
            })
        };
        match arg.as_str() {
            "--server" => server = Some(value()),
            "--listen" => listen = value(),
            "--dump" => dump_dir = Some(PathBuf::from(value())),
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            _ => {
                eprintln!("unknown argument {arg}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let Some(server) = server else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    let (server_host, server_port) = match server.rsplit_once(':') {
        Some((host, port)) => match port.parse() {
            Ok(port) => (host.to_string(), port),
            Err(_) => {
                eprintln!("bad port in {server}");
                std::process::exit(2);
            }
        },
        None => (server, 25565),
    };
    let listener = TcpListener::bind(&listen).unwrap_or_else(|e| {
        eprintln!("cannot listen on {listen}: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "[bridge] Horizon clients join {listen}; NeoForge server {server_host}:{server_port}"
    );
    let options = Arc::new(Options {
        server_host,
        server_port,
        dump_dir,
    });
    if let Err(e) = serve(listener, options) {
        eprintln!("[bridge] {e}");
        std::process::exit(1);
    }
}
