//! Block states from the server's numbering (1.21.1 plus its mods, sent by the
//! companion mod) to the client's (26.3 plus the content mods it loaded, its
//! `facts.json`): matched by name and properties.
use crate::neoforge::BlockStates;
use horizon_world::chunk::BlockState;
use horizon_world::registry::{BlockRegistry, BlockStateId};

/// How each server state found its client state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Same block, same properties.
    pub exact: usize,
    /// Same block; properties the client lacks or values it doesn't know were dropped.
    pub adjusted: usize,
    /// A block the client doesn't have: drawn as the placeholder.
    pub missing: usize,
}

/// Vanilla blocks renamed between 1.21.1 and 26.3.
const RENAMED: &[(&str, &str)] = &[("minecraft:chain", "minecraft:iron_chain")];

fn client_name(server: &str) -> &str {
    RENAMED
        .iter()
        .find(|(old, _)| *old == server)
        .map_or(server, |(_, new)| new)
}

pub struct StateMap {
    to_client: Vec<u32>,
    placeholder: u32,
    pub stats: Stats,
    /// Server blocks the client doesn't have, by name.
    pub missing_blocks: Vec<String>,
}

impl StateMap {
    /// `placeholder` stands in for blocks the client lacks; it must be in `client`.
    pub fn build(server: &BlockStates, client: &BlockRegistry, placeholder: &str) -> Option<Self> {
        let placeholder = client.default_state(placeholder)?.0;
        let mut map = StateMap {
            to_client: Vec::new(),
            placeholder,
            stats: Stats::default(),
            missing_blocks: Vec::new(),
        };
        for block in &server.blocks {
            let end = block.first_state as usize + block.states.len();
            if map.to_client.len() < end {
                map.to_client.resize(end, placeholder);
            }
            let name = client_name(&block.id);
            let Some(default) = client.default_state(name) else {
                map.stats.missing += block.states.len();
                map.missing_blocks.push(block.id.clone());
                continue;
            };
            for (i, values) in block.states.iter().enumerate() {
                let state = BlockState {
                    name: name.to_string(),
                    properties: block
                        .properties
                        .iter()
                        .zip(values)
                        .map(|((name, names), &v)| (name.clone(), names[v as usize].clone()))
                        .collect(),
                };
                let id = match client.id(&state) {
                    Some(id) => {
                        map.stats.exact += 1;
                        id
                    }
                    None => {
                        map.stats.adjusted += 1;
                        closest(client, default, &state)
                    }
                };
                map.to_client[block.first_state as usize + i] = id.0;
            }
        }
        Some(map)
    }

    /// The client's id for a server state id; unknown ids get the placeholder.
    pub fn map(&self, server: i32) -> i32 {
        let id = usize::try_from(server)
            .ok()
            .and_then(|i| self.to_client.get(i));
        id.copied().unwrap_or(self.placeholder) as i32
    }

    pub fn server_len(&self) -> usize {
        self.to_client.len()
    }
}

/// The client's default state of the block with every server property it can take.
fn closest(client: &BlockRegistry, default: BlockStateId, state: &BlockState) -> BlockStateId {
    let mut best = default;
    for (key, value) in &state.properties {
        let Some(mut candidate) = client.state(best).cloned() else {
            break;
        };
        if !candidate.properties.contains_key(key) {
            continue;
        }
        candidate.properties.insert(key.clone(), value.clone());
        if let Some(id) = client.id(&candidate) {
            best = id;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neoforge::BlockEntry;

    fn registry() -> BlockRegistry {
        // stone; lever (powered: true, false); new_block only on the client.
        let json = r#"{"states": [
            ["minecraft:air", true, false, "", "minecraft:air"],
            ["minecraft:stone", false, false, "", "minecraft:stone"],
            ["minecraft:lever", false, false, "", {"id": "minecraft:lever", "properties": {"powered": "true"}}],
            ["minecraft:lever", false, false, "", {"id": "minecraft:lever", "properties": {"powered": "false"}}]
        ], "defaults": {"minecraft:air": 0, "minecraft:stone": 1, "minecraft:lever": 3}}"#;
        BlockRegistry::from_facts_json(json.as_bytes()).unwrap()
    }

    fn entry(
        id: &str,
        first: i32,
        properties: &[(&str, &[&str])],
        states: Vec<Vec<u8>>,
    ) -> BlockEntry {
        BlockEntry {
            id: id.into(),
            first_state: first,
            default_state: first,
            properties: properties
                .iter()
                .map(|(n, v)| (n.to_string(), v.iter().map(|s| s.to_string()).collect()))
                .collect(),
            states,
        }
    }

    #[test]
    fn maps_by_name_and_properties() {
        let server = BlockStates {
            blocks: vec![
                entry("minecraft:air", 0, &[], vec![vec![]]),
                // The server lists lever's values the other way round and adds a property.
                entry(
                    "minecraft:lever",
                    1,
                    &[
                        ("face", &["wall", "floor"]),
                        ("powered", &["false", "true"]),
                    ],
                    vec![vec![0, 0], vec![0, 1], vec![1, 0], vec![1, 1]],
                ),
                entry("create:shaft", 5, &[], vec![vec![]]),
            ],
        };
        let map = StateMap::build(&server, &registry(), "minecraft:stone").unwrap();
        assert_eq!(map.map(0), 0);
        assert_eq!(map.map(1), 3); // powered=false
        assert_eq!(map.map(2), 2); // powered=true
        assert_eq!(map.map(5), 1); // missing block → stone
        assert_eq!(map.map(999), 1);
        assert_eq!(
            map.stats,
            Stats {
                exact: 1,
                adjusted: 4,
                missing: 1
            }
        );
        assert_eq!(map.missing_blocks, vec!["create:shaft".to_string()]);
    }
}
