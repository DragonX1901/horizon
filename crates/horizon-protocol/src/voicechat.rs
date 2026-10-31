//! Simple Voice Chat's protocol (de.maxhenkel.voicechat, compatibility versions 20 and 21: the
//! mod's 2.6 and 2.7 lines for 1.21.1), for joining its voice server from the game.
//!
//! The game asks for a secret on `voicechat:request_secret` with the server's compatibility
//! version; the server answers on `voicechat:secret` with an AES key and its UDP port. Voice
//! then travels over UDP: a magic byte, (from the client) the player's or connection's id, and
//! an AES-GCM sealed packet (`NetworkMessage`). Audio is Opus, 48 kHz mono, 20 ms frames.
//!
//! `horizon:voicechat` is the NeoForge bridge's note to the game that the server runs the mod:
//! its compatibility version and the host its UDP port is on.
use crate::buf::{Reader, Writer};
use crate::crypto::Aes128;
use crate::{Error, Result};

/// The bridge's payload: (compatibility version, server host).
pub const SERVER: &str = "horizon:voicechat";
pub const REQUEST_SECRET: &str = "voicechat:request_secret";
pub const SECRET: &str = "voicechat:secret";
/// `UpdateStatePacket`: the client's voice chat is disabled (true) or on.
pub const UPDATE_STATE: &str = "voicechat:update_state";

pub const SAMPLE_RATE: u32 = 48_000;
/// 20 ms at 48 kHz.
pub const FRAME_SIZE: usize = 960;
const MAGIC: u8 = 0xff;
const MAX_OPUS: usize = 1275;
const MAX_PACKET: usize = 4096;

pub fn write_server(compatibility: i32, host: &str) -> Vec<u8> {
    let mut w = Writer::new();
    w.i32(compatibility);
    w.string(host);
    w.data
}

pub fn read_server(data: &[u8]) -> Result<(i32, String)> {
    let mut r = Reader::new(data);
    Ok((r.i32()?, r.string(32767)?))
}

pub fn request_secret(compatibility: i32) -> Vec<u8> {
    let mut w = Writer::new();
    w.i32(compatibility);
    w.data
}

pub fn update_state(disabled: bool) -> Vec<u8> {
    let mut w = Writer::new();
    w.bool(disabled);
    w.data
}

/// `SecretPacket`.
#[derive(Clone, Debug, PartialEq)]
pub struct Secret {
    pub key: [u8; 16],
    /// 21 only: the id UDP packets carry in place of the player's.
    pub connection: Option<u128>,
    pub port: i32,
    pub player: u128,
    /// `ServerConfig.Codec`: VOIP, AUDIO, RESTRICTED_LOWDELAY.
    pub codec: u8,
    pub mtu: i32,
    pub distance: f64,
    /// Milliseconds between keep-alives.
    pub keep_alive: i32,
    pub groups: bool,
    /// Empty: the server's own address.
    pub host: String,
    pub allow_recording: bool,
}

impl Secret {
    pub fn read(data: &[u8], compatibility: i32) -> Result<Self> {
        let mut r = Reader::new(data);
        let key: [u8; 16] = r
            .bytes(16)?
            .try_into()
            .map_err(|_| Error::new("short key"))?;
        let connection = if compatibility >= 21 {
            Some(r.uuid()?)
        } else {
            None
        };
        Ok(Self {
            key,
            connection,
            port: r.i32()?,
            player: r.uuid()?,
            codec: r.u8()?,
            mtu: r.i32()?,
            distance: r.f64()?,
            keep_alive: r.i32()?,
            groups: r.bool()?,
            host: r.string(32767)?,
            allow_recording: r.bool()?,
        })
    }

    /// The id the client's UDP packets start with.
    pub fn sender(&self) -> u128 {
        self.connection.unwrap_or(self.player)
    }
}

/// A `NetworkMessage`'s packet.
#[derive(Clone, Debug, PartialEq)]
pub enum Packet {
    Mic {
        data: Vec<u8>,
        sequence: i64,
        whispering: bool,
    },
    PlayerSound {
        channel: u128,
        sender: u128,
        data: Vec<u8>,
        sequence: i64,
        distance: f32,
        whispering: bool,
        category: Option<String>,
    },
    GroupSound {
        channel: u128,
        sender: u128,
        data: Vec<u8>,
        sequence: i64,
        category: Option<String>,
    },
    LocationSound {
        channel: u128,
        sender: u128,
        pos: [f64; 3],
        data: Vec<u8>,
        sequence: i64,
        distance: f32,
        category: Option<String>,
    },
    /// 20: the player and the key; 21: empty.
    Authenticate {
        player: u128,
        key: [u8; 16],
    },
    /// 21: a nonce to send back in the connection check.
    AuthenticateAck {
        nonce: Option<[u8; 16]>,
    },
    Ping {
        id: u128,
        timestamp: i64,
    },
    KeepAlive,
    ConnectionCheck {
        nonce: Option<[u8; 16]>,
    },
    ConnectionCheckAck,
}

const WHISPER: u8 = 0b1;
const HAS_CATEGORY: u8 = 0b10;

fn nonce(r: &mut Reader) -> Result<Option<[u8; 16]>> {
    if r.remaining() < 16 {
        return Ok(None);
    }
    Ok(Some(
        r.bytes(16)?
            .try_into()
            .map_err(|_| Error::new("short nonce"))?,
    ))
}

impl Packet {
    fn read(r: &mut Reader) -> Result<Self> {
        let category = |r: &mut Reader, flags: u8| -> Result<Option<String>> {
            Ok(if flags & HAS_CATEGORY != 0 {
                Some(r.string(16)?)
            } else {
                None
            })
        };
        Ok(match r.u8()? {
            0x1 => Self::Mic {
                data: r.byte_array(MAX_OPUS)?,
                sequence: r.i64()?,
                whispering: r.bool()?,
            },
            0x2 => {
                let (channel, sender, data, sequence, distance) = (
                    r.uuid()?,
                    r.uuid()?,
                    r.byte_array(MAX_OPUS)?,
                    r.i64()?,
                    r.f32()?,
                );
                let flags = r.u8()?;
                Self::PlayerSound {
                    channel,
                    sender,
                    data,
                    sequence,
                    distance,
                    whispering: flags & WHISPER != 0,
                    category: category(r, flags)?,
                }
            }
            0x3 => {
                let (channel, sender, data, sequence) =
                    (r.uuid()?, r.uuid()?, r.byte_array(MAX_OPUS)?, r.i64()?);
                let flags = r.u8()?;
                Self::GroupSound {
                    channel,
                    sender,
                    data,
                    sequence,
                    category: category(r, flags)?,
                }
            }
            0x4 => {
                let (channel, sender) = (r.uuid()?, r.uuid()?);
                let pos = [r.f64()?, r.f64()?, r.f64()?];
                let (data, sequence, distance) = (r.byte_array(MAX_OPUS)?, r.i64()?, r.f32()?);
                let flags = r.u8()?;
                Self::LocationSound {
                    channel,
                    sender,
                    pos,
                    data,
                    sequence,
                    distance,
                    category: category(r, flags)?,
                }
            }
            0x6 => Self::AuthenticateAck { nonce: nonce(r)? },
            0x7 => Self::Ping {
                id: r.uuid()?,
                timestamp: r.i64()?,
            },
            0x8 => Self::KeepAlive,
            0xA => Self::ConnectionCheckAck,
            id => {
                return Err(Error::new(format!(
                    "voice packet {id} is not sent to clients"
                )));
            }
        })
    }

    fn write(&self, w: &mut Writer, compatibility: i32) {
        match self {
            Self::Mic {
                data,
                sequence,
                whispering,
            } => {
                w.u8(0x1);
                w.byte_array(data);
                w.i64(*sequence);
                w.bool(*whispering);
            }
            Self::Authenticate { player, key } => {
                w.u8(0x5);
                if compatibility < 21 {
                    w.uuid(*player);
                    w.bytes(key);
                }
            }
            Self::Ping { id, timestamp } => {
                w.u8(0x7);
                w.uuid(*id);
                w.i64(*timestamp);
            }
            Self::KeepAlive => w.u8(0x8),
            Self::ConnectionCheck { nonce } => {
                w.u8(0x9);
                if let Some(n) = nonce {
                    w.bytes(n);
                }
            }
            // Only the server sends the rest.
            _ => {}
        }
    }
}

/// The client's datagram for `packet`.
pub fn seal(secret: &Secret, compatibility: i32, packet: &Packet) -> Result<Vec<u8>> {
    let mut plain = Writer::new();
    packet.write(&mut plain, compatibility);
    let mut iv = [0u8; 12];
    crate::crypto::random_bytes(&mut iv)?;
    let mut payload = iv.to_vec();
    payload.extend(gcm_seal(&secret.key, &iv, &plain.data));
    let mut w = Writer::new();
    w.u8(MAGIC);
    w.uuid(secret.sender());
    w.byte_array(&payload);
    Ok(w.data)
}

/// A datagram from the voice server.
pub fn open(secret: &Secret, datagram: &[u8]) -> Result<Packet> {
    let mut r = Reader::new(datagram);
    if r.u8()? != MAGIC {
        return Err(Error::new("not a voice packet"));
    }
    let payload = r.byte_array(MAX_PACKET)?;
    if payload.len() < 12 + 16 {
        return Err(Error::new("short voice packet"));
    }
    let (iv, sealed) = payload.split_at(12);
    let plain = gcm_open(&secret.key, iv.try_into().unwrap(), sealed)
        .ok_or_else(|| Error::new("voice packet failed authentication"))?;
    Packet::read(&mut Reader::new(&plain))
}

// AES-128-GCM (NIST SP 800-38D) with a 96-bit IV, a 128-bit tag and no associated data:
// Java's "AES/GCM/NoPadding" as the mod uses it.

fn gf_mul(x: u128, y: u128) -> u128 {
    const R: u128 = 0xe1 << 120;
    let (mut z, mut v) = (0u128, y);
    for i in 0..128 {
        if x >> (127 - i) & 1 == 1 {
            z ^= v;
        }
        v = if v & 1 == 1 { (v >> 1) ^ R } else { v >> 1 };
    }
    z
}

fn ghash(h: u128, data: &[u8]) -> u128 {
    let mut y = 0u128;
    for chunk in data.chunks(16) {
        let mut block = [0u8; 16];
        block[..chunk.len()].copy_from_slice(chunk);
        y = gf_mul(y ^ u128::from_be_bytes(block), h);
    }
    // Lengths in bits: no associated data, then the ciphertext.
    gf_mul(y ^ (data.len() as u128 * 8), h)
}

fn ctr(aes: &Aes128, iv: &[u8; 12], data: &mut [u8]) {
    for (i, chunk) in data.chunks_mut(16).enumerate() {
        let mut block = [0u8; 16];
        block[..12].copy_from_slice(iv);
        block[12..].copy_from_slice(&(i as u32 + 2).to_be_bytes());
        aes.encrypt_block(&mut block);
        chunk.iter_mut().zip(block).for_each(|(b, k)| *b ^= k);
    }
}

fn tag(aes: &Aes128, iv: &[u8; 12], ciphertext: &[u8]) -> [u8; 16] {
    let mut h = [0u8; 16];
    aes.encrypt_block(&mut h);
    let mut j0 = [0u8; 16];
    j0[..12].copy_from_slice(iv);
    j0[15] = 1;
    aes.encrypt_block(&mut j0);
    (ghash(u128::from_be_bytes(h), ciphertext) ^ u128::from_be_bytes(j0)).to_be_bytes()
}

fn gcm_seal(key: &[u8; 16], iv: &[u8; 12], plain: &[u8]) -> Vec<u8> {
    let aes = Aes128::new(key);
    let mut out = plain.to_vec();
    ctr(&aes, iv, &mut out);
    let t = tag(&aes, iv, &out);
    out.extend(t);
    out
}

fn gcm_open(key: &[u8; 16], iv: &[u8; 12], sealed: &[u8]) -> Option<Vec<u8>> {
    let (ciphertext, expected) = sealed.split_at(sealed.len().checked_sub(16)?);
    let aes = Aes128::new(key);
    let t = tag(&aes, iv, ciphertext);
    // Constant time.
    if t.iter().zip(expected).fold(0, |d, (a, b)| d | (a ^ b)) != 0 {
        return None;
    }
    let mut out = ciphertext.to_vec();
    ctr(&aes, iv, &mut out);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn gcm_matches_the_nist_vectors() {
        // McGrew & Viega's GCM spec, test cases 2 and 3.
        let zero = [0u8; 16];
        assert_eq!(
            gcm_seal(&zero, &[0; 12], &[0; 16]),
            hex("0388dace60b6a392f328c2b971b2fe78ab6e47d42cec13bdf53a67b21257bddf")
        );
        let key: [u8; 16] = hex("feffe9928665731c6d6a8f9467308308").try_into().unwrap();
        let iv: [u8; 12] = hex("cafebabefacedbaddecaf888").try_into().unwrap();
        let plain = hex(
            "d9313225f88406e5a55909c5aff5269a86a7a9531534f7da2e4c303d8a318a721c3c0c95956809532fcf0e2449a6b525b16aedf5aa0de657ba637b391aafd255",
        );
        let sealed = gcm_seal(&key, &iv, &plain);
        assert_eq!(
            &sealed[plain.len()..],
            hex("4d5c2af327cd64a62cf35abd2ba6fab4").as_slice()
        );
        assert_eq!(gcm_open(&key, &iv, &sealed).unwrap(), plain);
        let mut bad = sealed.clone();
        bad[3] ^= 1;
        assert!(gcm_open(&key, &iv, &bad).is_none());
    }

    #[test]
    fn packets_round_trip_through_the_seal() {
        let secret = Secret {
            key: [7; 16],
            connection: None,
            port: 24454,
            player: 42,
            codec: 0,
            mtu: 1024,
            distance: 48.0,
            keep_alive: 1000,
            groups: true,
            host: String::new(),
            allow_recording: true,
        };
        let datagram = seal(&secret, 20, &Packet::KeepAlive).unwrap();
        // magic, player id, then the sealed payload: the server's own layout minus the id.
        let mut server = vec![MAGIC];
        server.extend_from_slice(&datagram[17..]);
        assert_eq!(open(&secret, &server).unwrap(), Packet::KeepAlive);
    }
}
