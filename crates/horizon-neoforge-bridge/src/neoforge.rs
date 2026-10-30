//! NeoForge 21.1's own configuration payloads (net.neoforged.neoforge.network.payload),
//! read and written the way its stream codecs do.
use horizon_protocol::buf::{Reader, Writer};
use horizon_protocol::{Error, Result};
use std::collections::BTreeMap;

/// `neoforge:register`: ModdedNetworkQueryPayload. The server sends it empty to ask
/// whether the client is modded; a NeoForge client answers with its channels.
pub const QUERY: &str = "neoforge:register";
/// `neoforge:network`: ModdedNetworkPayload, the negotiated channel setup.
pub const NETWORK: &str = "neoforge:network";
/// `neoforge:modded_network_setup_failed`.
pub const SETUP_FAILED: &str = "neoforge:modded_network_setup_failed";
pub const FROZEN_SYNC_START: &str = "neoforge:frozen_registry_sync_start";
pub const FROZEN_REGISTRY: &str = "neoforge:frozen_registry";
pub const FROZEN_SYNC_COMPLETED: &str = "neoforge:frozen_registry_sync_completed";
pub const KNOWN_DATA_MAPS: &str = "neoforge:known_registry_data_maps";
pub const KNOWN_DATA_MAPS_REPLY: &str = "neoforge:known_registry_data_maps_reply";
pub const EXTENSIBLE_ENUM_DATA: &str = "neoforge:extensible_enum_data";
pub const EXTENSIBLE_ENUM_ACK: &str = "neoforge:extensible_enum_ack";
pub const FEATURE_FLAGS: &str = "neoforge:feature_flags";
pub const FEATURE_FLAGS_ACK: &str = "neoforge:feature_flags_ack";
pub const CONFIG_FILE: &str = "neoforge:config_file";
pub const SPLIT: &str = "neoforge:split";
/// `minecraft:register` / `minecraft:unregister`: Dinnerbone's NUL-separated channel lists.
pub const MC_REGISTER: &str = "minecraft:register";
pub const MC_UNREGISTER: &str = "minecraft:unregister";

/// The channel the Horizon bridge companion mod listens for: a client that announces it
/// is negotiated as having every channel the server registers.
pub const BRIDGE_HELLO: &str = "horizon_bridge:hello";
/// The companion mod's block-state table.
pub const BRIDGE_BLOCK_STATES: &str = "horizon_bridge:block_states";

/// ConnectionProtocol ordinals in 1.21.1 (checked against the server jar).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Protocol {
    Handshaking = 0,
    Play = 1,
    Status = 2,
    Login = 3,
    Configuration = 4,
}
impl Protocol {
    pub fn from_ordinal(n: i32) -> Result<Self> {
        Ok(match n {
            0 => Self::Handshaking,
            1 => Self::Play,
            2 => Self::Status,
            3 => Self::Login,
            4 => Self::Configuration,
            _ => {
                return Err(Error::new(format!(
                    "Unknown ConnectionProtocol ordinal {n}"
                )));
            }
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            Self::Handshaking => "handshake",
            Self::Play => "play",
            Self::Status => "status",
            Self::Login => "login",
            Self::Configuration => "configuration",
        }
    }
}

/// PacketFlow ordinals in 1.21.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Serverbound = 0,
    Clientbound = 1,
}

/// ModdedNetworkQueryComponent: one channel a side offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryComponent {
    pub id: String,
    pub version: String,
    pub flow: Option<Flow>,
    pub optional: bool,
}
impl QueryComponent {
    fn new(id: &str, flow: Option<Flow>) -> Self {
        Self {
            id: id.into(),
            version: "1".into(),
            flow,
            optional: true,
        }
    }
}

/// The channels NeoForge registers itself (NetworkInitialization and
/// GenericPacketSplitter, all version "1" and optional), plus the companion mod's
/// marker. A NeoForge client offers the same, so an unmodded NeoForge server
/// negotiates with the bridge as with any NeoForge client.
pub fn bridge_channels() -> BTreeMap<Protocol, Vec<QueryComponent>> {
    use Flow::{Clientbound as C, Serverbound as S};
    let configuration = vec![
        QueryComponent::new(CONFIG_FILE, Some(C)),
        QueryComponent::new(FROZEN_SYNC_START, Some(C)),
        QueryComponent::new(FROZEN_REGISTRY, Some(C)),
        QueryComponent::new(FROZEN_SYNC_COMPLETED, None),
        QueryComponent::new(KNOWN_DATA_MAPS, Some(C)),
        QueryComponent::new(EXTENSIBLE_ENUM_DATA, Some(C)),
        QueryComponent::new(FEATURE_FLAGS, Some(C)),
        QueryComponent::new(KNOWN_DATA_MAPS_REPLY, Some(S)),
        QueryComponent::new(EXTENSIBLE_ENUM_ACK, Some(S)),
        QueryComponent::new(FEATURE_FLAGS_ACK, Some(S)),
        QueryComponent::new(SPLIT, None),
        QueryComponent::new(BRIDGE_HELLO, None),
        QueryComponent::new(BRIDGE_BLOCK_STATES, Some(C)),
    ];
    let play = [
        "neoforge:advanced_add_entity",
        "neoforge:advanced_open_screen",
        "neoforge:auxiliary_light_data",
        "neoforge:registry_data_map_sync",
        "neoforge:advanced_container_set_data",
        "neoforge:custom_time_packet",
        "neoforge:sync_attachments",
    ]
    .iter()
    .map(|id| QueryComponent::new(id, Some(C)))
    .chain([QueryComponent::new(SPLIT, None)])
    .collect();
    BTreeMap::from([
        (Protocol::Configuration, configuration),
        (Protocol::Play, play),
    ])
}

/// ModdedNetworkQueryPayload's codec: map<protocol ordinal, set<component>>.
pub fn write_query(w: &mut Writer, queries: &BTreeMap<Protocol, Vec<QueryComponent>>) {
    w.varint(queries.len() as i32);
    for (protocol, components) in queries {
        w.varint(*protocol as i32);
        w.varint(components.len() as i32);
        for c in components {
            w.string(&c.id);
            w.string(&c.version);
            match c.flow {
                Some(flow) => {
                    w.bool(true);
                    w.varint(flow as i32);
                }
                None => w.bool(false),
            }
            w.bool(c.optional);
        }
    }
}
pub fn read_query(r: &mut Reader) -> Result<BTreeMap<Protocol, Vec<QueryComponent>>> {
    let mut out = BTreeMap::new();
    for _ in 0..r.count(64)? {
        let protocol = Protocol::from_ordinal(r.varint()?)?;
        let mut components = Vec::new();
        for _ in 0..r.count(1 << 16)? {
            let id = r.string(32767)?;
            let version = r.string(32767)?;
            let flow = if r.bool()? {
                Some(match r.varint()? {
                    0 => Flow::Serverbound,
                    1 => Flow::Clientbound,
                    n => return Err(Error::new(format!("Unknown PacketFlow ordinal {n}"))),
                })
            } else {
                None
            };
            components.push(QueryComponent {
                id,
                version,
                flow,
                optional: r.bool()?,
            });
        }
        out.insert(protocol, components);
    }
    Ok(out)
}

/// NetworkPayloadSetup: the channels the server settled on, with their versions.
pub type Setup = BTreeMap<Protocol, BTreeMap<String, String>>;
pub fn read_setup(r: &mut Reader) -> Result<Setup> {
    let mut out = BTreeMap::new();
    for _ in 0..r.count(64)? {
        let protocol = Protocol::from_ordinal(r.varint()?)?;
        let mut channels = BTreeMap::new();
        for _ in 0..r.count(1 << 16)? {
            let key = r.string(32767)?;
            // NetworkChannel repeats the id before its chosen version.
            let _id = r.string(32767)?;
            channels.insert(key, r.string(32767)?);
        }
        out.insert(protocol, channels);
    }
    Ok(out)
}

/// DinnerboneProtocolUtils: identifiers separated (and ended) by NUL.
pub fn read_channels(data: &[u8]) -> Vec<String> {
    data.split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| s.iter().map(|&b| b as char).collect())
        .collect()
}
pub fn write_channels(channels: &[String]) -> Vec<u8> {
    let mut out = Vec::new();
    for c in channels {
        out.extend(c.chars().map(|ch| ch as u8));
        out.push(0);
    }
    out
}

/// FrozenRegistrySyncStartPayload: the registries about to be synced.
pub fn read_sync_start(r: &mut Reader) -> Result<Vec<String>> {
    (0..r.count(1 << 16)?).map(|_| r.string(32767)).collect()
}

/// A static registry as the server froze it (RegistrySnapshot).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RegistrySnapshot {
    /// Network id → name, in id order.
    pub ids: BTreeMap<i32, String>,
    /// Old name → current name.
    pub aliases: BTreeMap<String, String>,
}
/// FrozenRegistryPayload: the registry's name, then its snapshot.
pub fn read_frozen_registry(r: &mut Reader) -> Result<(String, RegistrySnapshot)> {
    let name = r.string(32767)?;
    let mut snapshot = RegistrySnapshot::default();
    for _ in 0..r.count(1 << 24)? {
        let id = r.varint()?;
        snapshot.ids.insert(id, r.string(32767)?);
    }
    for _ in 0..r.count(1 << 24)? {
        let from = r.string(32767)?;
        snapshot.aliases.insert(from, r.string(32767)?);
    }
    Ok((name, snapshot))
}

/// KnownRegistryDataMapsPayload: registry → [(data map id, mandatory)].
pub fn read_known_data_maps(r: &mut Reader) -> Result<BTreeMap<String, Vec<(String, bool)>>> {
    let mut out = BTreeMap::new();
    for _ in 0..r.count(1 << 16)? {
        let registry = r.string(32767)?;
        let maps = (0..r.count(1 << 16)?)
            .map(|_| Ok((r.string(32767)?, r.bool()?)))
            .collect::<Result<Vec<_>>>()?;
        out.insert(registry, maps);
    }
    Ok(out)
}
/// KnownRegistryDataMapsReplyPayload: the data maps the client accepts. The bridge
/// accepts every one, mandatory or not, so no mandatory map can refuse it.
pub fn write_known_data_maps_reply(w: &mut Writer, maps: &BTreeMap<String, Vec<(String, bool)>>) {
    w.varint(maps.len() as i32);
    for (registry, ids) in maps {
        w.string(registry);
        w.varint(ids.len() as i32);
        for (id, _) in ids {
            w.string(id);
        }
    }
}

/// FeatureFlagDataPayload: the modded feature flags.
pub fn read_feature_flags(r: &mut Reader) -> Result<Vec<String>> {
    (0..r.count(1 << 16)?).map(|_| r.string(32767)).collect()
}

/// One CheckExtensibleEnums.EnumEntry: the class, its NetworkCheck and, when mods
/// extended it, (vanilla count, total count, added names).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnumEntry {
    pub class_name: String,
    pub network_check: String,
    pub extension: Option<(i32, i32, Vec<String>)>,
}
pub fn read_extensible_enums(r: &mut Reader) -> Result<Vec<EnumEntry>> {
    let mut out = Vec::new();
    for _ in 0..r.count(1 << 16)? {
        let class_name = r.string(32767)?;
        let network_check = r.string(32767)?;
        let extension = if r.bool()? {
            let vanilla = r.varint()?;
            let total = r.varint()?;
            let names = (0..r.count(1 << 16)?)
                .map(|_| r.string(32767))
                .collect::<Result<_>>()?;
            Some((vanilla, total, names))
        } else {
            None
        };
        out.push(EnumEntry {
            class_name,
            network_check,
            extension,
        });
    }
    Ok(out)
}

/// ConfigFilePayload: a synced config file (name, contents).
pub fn read_config_file(r: &mut Reader) -> Result<(String, Vec<u8>)> {
    let name = r.string(32767)?;
    let len = r.count(usize::MAX)?;
    Ok((name, r.bytes(len)?.to_vec()))
}

/// GenericPacketSplitter's reassembly: parts start with 1 (first), 0 (middle) or 2
/// (last); the joined parts are one whole packet body (id + payload).
#[derive(Default)]
pub struct Splitter {
    parts: Vec<u8>,
}
impl Splitter {
    /// The split payload is ByteArray-prefixed (NeoForgeStreamCodecs.UNBOUNDED_BYTE_ARRAY).
    pub fn accept(&mut self, payload: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut r = Reader::new(payload);
        let len = r.count(usize::MAX)?;
        let part = r.bytes(len)?;
        let (&state, rest) = part
            .split_first()
            .ok_or_else(|| Error::new("Empty neoforge:split part"))?;
        if state == 1 {
            self.parts.clear();
        }
        self.parts.extend_from_slice(rest);
        if self.parts.len() > 64 << 20 {
            return Err(Error::new("neoforge:split packet over 64 MiB"));
        }
        Ok((state == 2).then(|| std::mem::take(&mut self.parts)))
    }
}

/// The companion mod's block-state table: per block in registry order, its id, its
/// properties with their values in declaration order, and for each of its states
/// (from the first state id on) the index of each property's value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockStates {
    pub blocks: Vec<BlockEntry>,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlockEntry {
    pub id: String,
    pub first_state: i32,
    pub default_state: i32,
    pub properties: Vec<(String, Vec<String>)>,
    /// One row per state: the value index of each property, in `properties` order.
    pub states: Vec<Vec<u8>>,
}
impl BlockStates {
    pub fn state_count(&self) -> usize {
        self.blocks.iter().map(|b| b.states.len()).sum()
    }
}
pub fn read_block_states(r: &mut Reader) -> Result<BlockStates> {
    let mut blocks = Vec::new();
    for _ in 0..r.count(1 << 20)? {
        let id = r.string(32767)?;
        let first_state = r.varint()?;
        let default_state = r.varint()?;
        let mut properties = Vec::new();
        for _ in 0..r.count(256)? {
            let name = r.string(32767)?;
            let values = (0..r.count(256)?)
                .map(|_| r.string(32767))
                .collect::<Result<_>>()?;
            properties.push((name, values));
        }
        let n = r.count(1 << 20)?;
        let mut states = Vec::with_capacity(n);
        for _ in 0..n {
            states.push(r.bytes(properties.len())?.to_vec());
        }
        blocks.push(BlockEntry {
            id,
            first_state,
            default_state,
            properties,
            states,
        });
    }
    Ok(BlockStates { blocks })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_round_trip() {
        let queries = bridge_channels();
        let mut w = Writer::new();
        write_query(&mut w, &queries);
        let mut r = Reader::new(&w.data);
        assert_eq!(read_query(&mut r).unwrap(), queries);
        assert_eq!(r.remaining(), 0);
    }

    #[test]
    fn query_wire_layout() {
        let queries = BTreeMap::from([(
            Protocol::Configuration,
            vec![QueryComponent::new("a:b", Some(Flow::Clientbound))],
        )]);
        let mut w = Writer::new();
        write_query(&mut w, &queries);
        // map size 1, CONFIGURATION (4), set size 1, "a:b", "1", present, CLIENTBOUND (1), optional.
        assert_eq!(w.data, [1, 4, 1, 3, b'a', b':', b'b', 1, b'1', 1, 1, 1]);
    }

    #[test]
    fn dinnerbone_channels() {
        let channels = vec!["a:b".to_string(), "c:d".to_string()];
        let bytes = write_channels(&channels);
        assert_eq!(bytes, b"a:b\0c:d\0");
        assert_eq!(read_channels(&bytes), channels);
        assert_eq!(read_channels(b"a:b"), vec!["a:b".to_string()]);
    }

    #[test]
    fn frozen_registry() {
        let mut w = Writer::new();
        w.string("minecraft:block");
        w.varint(2);
        w.varint(0);
        w.string("minecraft:air");
        w.varint(1);
        w.string("minecraft:stone");
        w.varint(1);
        w.string("old:rock");
        w.string("minecraft:stone");
        let (name, snapshot) = read_frozen_registry(&mut Reader::new(&w.data)).unwrap();
        assert_eq!(name, "minecraft:block");
        assert_eq!(snapshot.ids[&1], "minecraft:stone");
        assert_eq!(snapshot.aliases["old:rock"], "minecraft:stone");
    }

    #[test]
    fn split_reassembly() {
        let mut s = Splitter::default();
        let part = |state: u8, bytes: &[u8]| {
            let mut body = vec![state];
            body.extend_from_slice(bytes);
            let mut w = Writer::new();
            w.byte_array(&body);
            w.data
        };
        assert_eq!(s.accept(&part(1, b"ab")).unwrap(), None);
        assert_eq!(s.accept(&part(0, b"cd")).unwrap(), None);
        assert_eq!(s.accept(&part(2, b"e")).unwrap(), Some(b"abcde".to_vec()));
    }

    #[test]
    fn block_states() {
        let mut w = Writer::new();
        w.varint(1);
        w.string("minecraft:lever");
        w.varint(10);
        w.varint(11);
        w.varint(1);
        w.string("powered");
        w.varint(2);
        w.string("true");
        w.string("false");
        w.varint(2);
        w.u8(0);
        w.u8(1);
        let states = read_block_states(&mut Reader::new(&w.data)).unwrap();
        assert_eq!(states.state_count(), 2);
        assert_eq!(states.blocks[0].states[1], vec![1]);
    }
}
