//! `horizon-neoforge-host --server-dir DIR --world DIR [--resources DIR] [--java PATH] [--name NAME]`:
//! a modded single-player world, started and driven by the game (see `host.rs`). Without
//! `--resources`, the game sends them on stdin once loaded.
use horizon_neoforge_bridge::host::{HostOptions, run};
use std::path::PathBuf;

const USAGE: &str = "usage: horizon-neoforge-host --server-dir DIR --world DIR [--resources DIR] [--java PATH] [--name NAME] [--owner PLAYER]";

fn main() {
    let (mut server_dir, mut world, mut resources) = (None, None, None);
    let mut java = PathBuf::from("java");
    let mut name = None;
    let mut owner = None;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let Some(value) = args.next() else {
            eprintln!("{arg} needs a value\n{USAGE}");
            std::process::exit(2);
        };
        match arg.as_str() {
            "--server-dir" => server_dir = Some(PathBuf::from(value)),
            "--world" => world = Some(PathBuf::from(value)),
            "--resources" => resources = Some(PathBuf::from(value)),
            "--java" => java = PathBuf::from(value),
            "--name" => name = Some(value),
            "--owner" => owner = Some(value),
            _ => {
                eprintln!("unknown argument {arg}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }
    let (Some(server_dir), Some(world)) = (server_dir, world) else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    let name = name.unwrap_or_else(|| {
        world
            .file_name()
            .map_or("World".into(), |n| n.to_string_lossy().into_owned())
    });
    if let Err(e) = run(HostOptions {
        server_dir,
        world,
        resources,
        java,
        name,
        owner,
    }) {
        eprintln!("[host] {e}");
        std::process::exit(1);
    }
}
