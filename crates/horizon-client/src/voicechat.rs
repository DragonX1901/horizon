//! Simple Voice Chat in the game: when the server runs the mod (the NeoForge bridge says so on
//! `horizon:voicechat`), the game asks for a secret, joins the mod's UDP voice server, sends the
//! microphone while the push-to-talk key is held, and plays what others say, quieter with
//! distance (de.maxhenkel.voicechat's ClientVoicechat; the wire format is
//! `horizon_protocol::voicechat`).
use crate::voice_audio::{Playback, VoiceAudio};
use horizon_protocol::voicechat::{self as svc, Packet, Secret};
use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Where the listener and the other players are, for each voice's volume.
#[derive(Default)]
pub struct Positions {
    pub listener: [f64; 3],
    pub players: HashMap<u128, [f64; 3]>,
}

/// One server's voice chat, from the bridge's note until the player leaves.
pub struct VoiceChat {
    pub compatibility: i32,
    host: String,
    /// Asked for the secret yet (once play has begun).
    pub requested: bool,
    /// The push-to-talk key is down.
    pub talking: Arc<AtomicBool>,
    pub positions: Arc<Mutex<Positions>>,
    /// Set once the voice server acknowledged the connection.
    pub connected: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
    audio: Option<VoiceAudio>,
}

impl VoiceChat {
    pub fn new(compatibility: i32, host: String) -> Self {
        Self {
            compatibility,
            host,
            requested: false,
            talking: Arc::new(AtomicBool::new(false)),
            positions: Arc::default(),
            connected: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(true)),
            audio: None,
        }
    }

    /// The `voicechat:request_secret` payload.
    pub fn request(&self) -> Vec<u8> {
        svc::request_secret(self.compatibility)
    }

    /// The server's secret arrived: open the sound streams and the voice connection.
    pub fn connect(&mut self, data: &[u8]) -> Result<(), String> {
        let secret = Secret::read(data, self.compatibility).map_err(|e| e.to_string())?;
        let host = if secret.host.is_empty() {
            self.host.clone()
        } else {
            secret.host.clone()
        };
        // The host may name a port of its own ("host:port").
        let target = match host.rsplit_once(':') {
            Some((h, p)) if p.parse::<u16>().is_ok() && !h.contains(':') => host.clone(),
            _ => format!("{host}:{}", secret.port),
        };
        let socket = UdpSocket::bind(("0.0.0.0", 0)).map_err(|e| e.to_string())?;
        socket
            .connect(&target)
            .map_err(|e| format!("voice server {target}: {e}"))?;
        socket
            .set_read_timeout(Some(Duration::from_millis(250)))
            .map_err(|e| e.to_string())?;
        let audio = VoiceAudio::start()
            .inspect_err(|e| log::warn!("Heisen: voice chat without sound: {e}"))
            .ok();
        let playback = audio.as_ref().map(|a| a.playback.clone());
        let mut audio = audio;
        let microphone = audio.as_mut().and_then(|a| a.frames.take());
        self.audio = audio;
        let link = Arc::new(Link {
            socket,
            secret,
            compatibility: self.compatibility,
        });
        log::info!("Heisen: voice chat at {target}");
        let (l, running, connected, positions) = (
            link.clone(),
            self.running.clone(),
            self.connected.clone(),
            self.positions.clone(),
        );
        std::thread::Builder::new()
            .name("voice-receive".into())
            .spawn(move || l.receive(&running, &connected, &positions, playback))
            .map_err(|e| e.to_string())?;
        if let Some(frames) = microphone {
            let (running, connected, talking) = (
                self.running.clone(),
                self.connected.clone(),
                self.talking.clone(),
            );
            std::thread::Builder::new()
                .name("voice-send".into())
                .spawn(move || link.send_microphone(frames, &running, &connected, &talking))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

impl Drop for VoiceChat {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

struct Link {
    socket: UdpSocket,
    secret: Secret,
    compatibility: i32,
}

impl Link {
    fn send(&self, packet: &Packet) {
        match svc::seal(&self.secret, self.compatibility, packet) {
            Ok(datagram) => {
                let _ = self.socket.send(&datagram);
            }
            Err(e) => log::warn!("Heisen: voice packet not sent: {e}"),
        }
    }

    /// ClientVoicechatConnection: authenticate, check the connection, answer keep-alives and
    /// pings, and play sound packets.
    fn receive(
        &self,
        running: &AtomicBool,
        connected: &AtomicBool,
        positions: &Mutex<Positions>,
        playback: Option<Playback>,
    ) {
        let mut authenticated = false;
        let mut nonce = None;
        let mut last_try = Instant::now() - Duration::from_secs(1);
        let mut last_keep_alive = Instant::now();
        let mut decoders: HashMap<u128, Option<Decoder>> = HashMap::new();
        let mut buf = [0u8; 4096];
        let timeout = Duration::from_millis(self.secret.keep_alive.max(1000) as u64 * 10);
        while running.load(Ordering::SeqCst) {
            if !connected.load(Ordering::SeqCst) && last_try.elapsed() >= Duration::from_secs(1) {
                last_try = Instant::now();
                self.send(&if authenticated {
                    Packet::ConnectionCheck { nonce }
                } else {
                    Packet::Authenticate {
                        player: self.secret.player,
                        key: self.secret.key,
                    }
                });
            }
            if connected.load(Ordering::SeqCst) && last_keep_alive.elapsed() > timeout {
                log::warn!("Heisen: the voice server stopped answering");
                connected.store(false, Ordering::SeqCst);
                authenticated = false;
            }
            let Ok(n) = self.socket.recv(&mut buf) else {
                continue;
            };
            let Ok(packet) = svc::open(&self.secret, &buf[..n]) else {
                continue;
            };
            match packet {
                Packet::AuthenticateAck { nonce: n } => {
                    authenticated = true;
                    nonce = n;
                }
                Packet::ConnectionCheckAck => {
                    if authenticated && !connected.swap(true, Ordering::SeqCst) {
                        log::info!("Heisen: voice chat connected");
                        last_keep_alive = Instant::now();
                    }
                }
                Packet::KeepAlive => {
                    last_keep_alive = Instant::now();
                    self.send(&Packet::KeepAlive);
                }
                ping @ Packet::Ping { .. } => self.send(&ping),
                Packet::PlayerSound {
                    sender,
                    data,
                    distance,
                    ..
                } => {
                    let gain = positions
                        .lock()
                        .ok()
                        .and_then(|p| Some(falloff(p.listener, *p.players.get(&sender)?, distance)))
                        .unwrap_or(1.0);
                    play(&mut decoders, playback.as_ref(), sender, &data, gain);
                }
                Packet::LocationSound {
                    sender,
                    pos,
                    data,
                    distance,
                    ..
                } => {
                    let gain = positions
                        .lock()
                        .map_or(1.0, |p| falloff(p.listener, pos, distance));
                    play(&mut decoders, playback.as_ref(), sender, &data, gain);
                }
                Packet::GroupSound { sender, data, .. } => {
                    play(&mut decoders, playback.as_ref(), sender, &data, 1.0)
                }
                _ => {}
            }
        }
    }

    /// The microphone's frames, encoded and sent while the key is held.
    fn send_microphone(
        &self,
        frames: std::sync::mpsc::Receiver<Vec<i16>>,
        running: &AtomicBool,
        connected: &AtomicBool,
        talking: &AtomicBool,
    ) {
        let Some(mut encoder) = Encoder::new(self.secret.codec) else {
            return;
        };
        let mut sequence = 0i64;
        while running.load(Ordering::SeqCst) {
            let Ok(frame) = frames.recv_timeout(Duration::from_millis(250)) else {
                continue;
            };
            if !connected.load(Ordering::SeqCst) || !talking.load(Ordering::SeqCst) {
                continue;
            }
            if let Some(data) = encoder.encode(&frame) {
                self.send(&Packet::Mic {
                    data,
                    sequence,
                    whispering: false,
                });
                sequence += 1;
            }
        }
    }
}

/// The mod's volume over distance: full up close, silent at `distance`.
fn falloff(listener: [f64; 3], speaker: [f64; 3], distance: f32) -> f32 {
    let d = listener
        .iter()
        .zip(speaker)
        .map(|(a, b)| (a - b) * (a - b))
        .sum::<f64>()
        .sqrt();
    let max = f64::from(distance.max(1.0));
    (1.0 - d / max).clamp(0.0, 1.0) as f32
}

fn play(
    decoders: &mut HashMap<u128, Option<Decoder>>,
    playback: Option<&Playback>,
    sender: u128,
    data: &[u8],
    gain: f32,
) {
    let Some(playback) = playback else { return };
    let Some(decoder) = decoders.entry(sender).or_insert_with(Decoder::new).as_mut() else {
        return;
    };
    if let Some(pcm) = decoder.decode(data) {
        playback.push(sender, &pcm, gain);
    }
}

#[cfg(feature = "audio")]
struct Encoder(opus::Encoder);
#[cfg(feature = "audio")]
impl Encoder {
    fn new(codec: u8) -> Option<Self> {
        let application = match codec {
            1 => opus::Application::Audio,
            2 => opus::Application::LowDelay,
            _ => opus::Application::Voip,
        };
        opus::Encoder::new(svc::SAMPLE_RATE, opus::Channels::Mono, application)
            .inspect_err(|e| log::warn!("Heisen: no Opus encoder: {e}"))
            .ok()
            .map(Self)
    }
    fn encode(&mut self, frame: &[i16]) -> Option<Vec<u8>> {
        let mut out = vec![0u8; 1275];
        let n = self.0.encode(frame, &mut out).ok()?;
        out.truncate(n);
        Some(out)
    }
}

#[cfg(feature = "audio")]
struct Decoder(opus::Decoder);
#[cfg(feature = "audio")]
impl Decoder {
    fn new() -> Option<Self> {
        opus::Decoder::new(svc::SAMPLE_RATE, opus::Channels::Mono)
            .ok()
            .map(Self)
    }
    fn decode(&mut self, data: &[u8]) -> Option<Vec<i16>> {
        let mut pcm = vec![0i16; svc::FRAME_SIZE * 6];
        let n = self.0.decode(data, &mut pcm, false).ok()?;
        pcm.truncate(n);
        Some(pcm)
    }
}

// Without sound there is nothing to encode or play.
#[cfg(not(feature = "audio"))]
struct Encoder;
#[cfg(not(feature = "audio"))]
impl Encoder {
    fn new(_: u8) -> Option<Self> {
        None
    }
    fn encode(&mut self, _: &[i16]) -> Option<Vec<u8>> {
        None
    }
}
#[cfg(not(feature = "audio"))]
struct Decoder;
#[cfg(not(feature = "audio"))]
impl Decoder {
    fn new() -> Option<Self> {
        None
    }
    fn decode(&mut self, _: &[u8]) -> Option<Vec<i16>> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voices_fade_out_at_the_servers_distance() {
        assert_eq!(falloff([0.0; 3], [0.0; 3], 48.0), 1.0);
        assert!((falloff([0.0; 3], [24.0, 0.0, 0.0], 48.0) - 0.5).abs() < 1e-6);
        assert_eq!(falloff([0.0; 3], [100.0, 0.0, 0.0], 48.0), 0.0);
    }

    #[cfg(feature = "audio")]
    #[test]
    fn opus_round_trips_a_frame() {
        let mut e = Encoder::new(0).unwrap();
        let mut d = Decoder::new().unwrap();
        let frame: Vec<i16> = (0..960)
            .map(|i| ((i as f32 * 0.05).sin() * 8000.0) as i16)
            .collect();
        let data = e.encode(&frame).unwrap();
        assert_eq!(d.decode(&data).unwrap().len(), 960);
    }
}
