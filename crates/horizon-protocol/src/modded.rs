//! `horizon:modded_entities`: a configuration payload naming the entity types a
//! server adds beyond the client's registry (mods, through the NeoForge
//! bridge), with the ids it sends them under and their sizes, so the client
//! can spawn and draw them.
use crate::{Result, buf::Reader, buf::Writer};

pub const CHANNEL: &str = "horizon:modded_entities";

#[derive(Clone, Debug, PartialEq)]
pub struct ModdedEntity {
    /// The id the server's packets use for it.
    pub id: i32,
    pub name: String,
    pub width: f32,
    pub height: f32,
    pub eye_height: f32,
}

pub fn write(entities: &[ModdedEntity]) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(entities.len() as i32);
    for e in entities {
        w.varint(e.id);
        w.string(&e.name);
        w.f32(e.width);
        w.f32(e.height);
        w.f32(e.eye_height);
    }
    w.data
}

pub fn read(data: &[u8]) -> Result<Vec<ModdedEntity>> {
    let mut r = Reader::new(data);
    (0..r.count(1 << 16)?)
        .map(|_| {
            Ok(ModdedEntity {
                id: r.varint()?,
                name: r.string(32767)?,
                width: r.f32()?,
                height: r.f32()?,
                eye_height: r.f32()?,
            })
        })
        .collect()
}

/// `horizon:modded_particles`: particle types the server's mods add, by the id the bridge
/// sends them under (past the client's registry), so the client can draw them with the mod's
/// own sprites.
pub const PARTICLES: &str = "horizon:modded_particles";
/// Where the bridge numbers mods' particle types from.
pub const FIRST_PARTICLE: i32 = 1 << 14;

pub fn write_names(names: &[(i32, String)]) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(names.len() as i32);
    for (id, name) in names {
        w.varint(*id);
        w.string(name);
    }
    w.data
}

pub fn read_names(data: &[u8]) -> Result<Vec<(i32, String)>> {
    let mut r = Reader::new(data);
    (0..r.count(1 << 16)?)
        .map(|_| Ok((r.varint()?, r.string(32767)?)))
        .collect()
}

/// `horizon:block_speeds`: how fast mods' turning blocks turn (a block entity's `Speed`, in
/// Create's RPM), by position: (x, y, z, speed). The game turns their models.
pub const BLOCK_SPEEDS: &str = "horizon:block_speeds";

pub fn write_speeds(speeds: &[([i32; 3], f32)]) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(speeds.len() as i32);
    for (p, s) in speeds {
        p.iter().for_each(|&c| w.i32(c));
        w.f32(*s);
    }
    w.data
}

pub fn read_speeds(data: &[u8]) -> Result<Vec<([i32; 3], f32)>> {
    let mut r = Reader::new(data);
    (0..r.count(1 << 20)?)
        .map(|_| Ok(([r.i32()?, r.i32()?, r.i32()?], r.f32()?)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let list = vec![ModdedEntity {
            id: 200,
            name: "cobblemon:pokemon".into(),
            width: 0.6,
            height: 0.9,
            eye_height: 0.8,
        }];
        assert_eq!(read(&write(&list)).unwrap(), list);
    }
}
