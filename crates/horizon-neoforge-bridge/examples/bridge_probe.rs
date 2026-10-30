//! Join a bridge the way the Horizon client does (protocol 777) and print what it
//! answers: the status, then the configuration packets until the disconnect.
//! `cargo run -p horizon-neoforge-bridge --example bridge_probe -- 127.0.0.1:25566`
use horizon_protocol::buf::Reader;
use horizon_protocol::connection::Connection;
use horizon_protocol::ids::configuration::clientbound as cb;
use horizon_protocol::packets::configuration::ClientInformation;
use horizon_protocol::packets::{Empty, configuration, handshake, login, status};
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
        println!("configuration packet {id}");
    }
}
