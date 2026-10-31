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
