//! Join a bridge the way the Horizon client does (protocol 777) and print what it
//! answers: the status, then the configuration packets until the disconnect.
//! `cargo run -p horizon-neoforge-bridge --example bridge_probe -- 127.0.0.1:25566`
use horizon_protocol::buf::Reader;
use horizon_protocol::connection::Connection;
use horizon_protocol::ids::configuration::clientbound as cb;
use horizon_protocol::packets::configuration::ClientInformation;
use horizon_protocol::packets::{Empty, configuration, handshake, login, play, status};
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:25566".into());
    let (host, port) = addr.rsplit_once(':').ok_or("HOST:PORT")?;
    let port: u16 = port.parse()?;
    let intention = |intent| {
        handshake::Serverbound::Intention(handshake::Intention {
            protocol: horizon_protocol::PROTOCOL_VERSION,
            host: host.into(),
            port,
            intent,
        })
        .encode()
    };

    let mut c = Connection::connect(host, port, Duration::from_secs(5))?;
    c.send(&intention(handshake::STATUS))?;
    c.send(&status::Serverbound::StatusRequest(Empty).encode())?;
    let (_, body) = c.recv()?;
    println!("status: {}", Reader::new(&body).string(32767)?);

    let mut c = Connection::connect(host, port, Duration::from_secs(5))?;
    c.send(&intention(handshake::LOGIN))?;
    c.send(
        &login::Serverbound::Hello(login::ServerHello {
            name: "HorizonProbe".into(),
            uuid: 1,
        })
        .encode(),
    )?;
    loop {
        let (id, body) = c.recv()?;
        match login::Clientbound::decode(id, &body)? {
            login::Clientbound::LoginFinished(p) => {
                println!("login finished as {}", p.profile.name);
                c.send(&login::Serverbound::LoginAcknowledged(Empty).encode())?;
                break;
            }
            login::Clientbound::Disconnect(p) => {
                println!("login disconnect: {}", p.reason_json);
                return Ok(());
            }
            other => println!("login: {other:?}"),
        }
    }
    c.send(&configuration::Serverbound::ClientInformation(ClientInformation::new(8)).encode())?;
    loop {
        let (id, body) = c.recv()?;
        if id == cb::DISCONNECT {
            let reason = Reader::new(&body).nbt()?;
            println!("configuration disconnect: {reason:?}");
            return Ok(());
        }
        if id == cb::FINISH_CONFIGURATION {
            c.send(&configuration::Serverbound::FinishConfiguration(Empty).encode())?;
            break;
        }
        println!(
            "configuration packet {}",
            horizon_protocol::ids::configuration::clientbound::NAMES[id as usize]
        );
    }
    // Play: decode every packet as the client would, answer what the server waits for.
    let mut seen = std::collections::BTreeMap::<&str, usize>::new();
    let mut chunks = 0;
    while chunks < 40 {
        let (id, body) = c.recv()?;
        let name = horizon_protocol::ids::play::clientbound::NAMES[id as usize];
        *seen.entry(name).or_default() += 1;
        match play::Clientbound::decode(id, &body) {
            Ok(play::Clientbound::Unknown(..)) => {}
            Ok(play::Clientbound::KeepAlive(k)) => {
                c.send(&play::Serverbound::KeepAlive(k).encode())?
            }
            Ok(play::Clientbound::PlayerPosition(p)) => {
                println!("teleport to {:?}", p.position);
                c.send(
                    &play::Serverbound::AcceptTeleportation(play::AcceptTeleportation {
                        id: p.teleport,
                        x: p.position[0],
                        y: p.position[1],
                        z: p.position[2],
                        yaw: p.yaw,
                        pitch: p.pitch,
                    })
                    .encode(),
                )?;
            }
            Ok(play::Clientbound::LevelChunkWithLight(ch)) => {
                if chunks == 0 {
                    let sections = ch.chunk.sections(24, 16, 6)?;
                    let ids: std::collections::BTreeSet<i32> = sections
                        .iter()
                        .flat_map(|s| (0..4096).map(|i| s.states.get(i)))
                        .collect();
                    println!(
                        "first chunk {},{}: {} sections, state ids {:?}",
                        ch.x,
                        ch.z,
                        sections.len(),
                        ids
                    );
                }
                chunks += 1;
            }
            Ok(play::Clientbound::ChunkBatchFinished(_)) => c.send(
                &play::Serverbound::ChunkBatchReceived(play::ChunkBatchReceived {
                    chunks_per_tick: 10.0,
                })
                .encode(),
            )?,
            Ok(play::Clientbound::Disconnect(d)) => {
                println!("play disconnect: {d:?}");
                break;
            }
            Ok(_) => {}
            Err(e) => println!("DECODE FAILED {name}: {e}"),
        }
    }
    println!("play packets: {seen:?}");
    // PROBE_HOLD=seconds: stay in the world (answering keep-alives) before leaving.
    let hold: u64 = std::env::var("PROBE_HOLD")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let until = std::time::Instant::now() + Duration::from_secs(hold);
    c.reader
        .set_read_timeout(Some(Duration::from_millis(500)))?;
    while std::time::Instant::now() < until {
        if let Ok((id, body)) = c.recv()
            && let Ok(play::Clientbound::KeepAlive(k)) = play::Clientbound::decode(id, &body)
        {
            c.send(&play::Serverbound::KeepAlive(k).encode())?;
        }
    }
    Ok(())
}
