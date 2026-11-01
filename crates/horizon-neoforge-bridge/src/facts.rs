//! What a NeoForge server told the bridge during configuration, kept for the
//! client side: which mods' channels it runs, its frozen registries (blocks,
//! items, entity types... with their network ids) and its block states.
use crate::neoforge::{BlockStates, EnumEntry, Protocol, RegistrySnapshot, Setup};
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug, Default)]
pub struct ServerFacts {
    pub brand: Option<String>,
    /// The negotiated channels, or None when the server never negotiated (vanilla).
    pub setup: Option<Setup>,
    /// `minecraft:register` channels the server listens on.
    pub listening: Vec<String>,
    pub enabled_features: Vec<String>,
    pub modded_feature_flags: Vec<String>,
    pub extensible_enums: Vec<EnumEntry>,
    pub data_maps: BTreeMap<String, Vec<(String, bool)>>,
    pub config_files: Vec<String>,
    pub frozen: BTreeMap<String, RegistrySnapshot>,
    /// Data-driven registries (registry_data): registry → entry names, and
    /// whether the server sent each entry's data (it does not for known packs).
    pub registries: BTreeMap<String, Vec<(String, bool)>>,
    /// The packs the server offered as known (namespace, id, version).
    pub known_packs: Vec<(String, String, String)>,
    pub tag_registries: usize,
    pub block_states: Option<BlockStates>,
    /// Mod entity types: (name, width, height, eye height, the vanilla type it extends or "").
    pub entity_types: Vec<(String, f32, f32, f32, String)>,
    /// Simple Voice Chat's compatibility version, when the server runs it.
    pub voicechat: Option<i32>,
}

impl ServerFacts {
    /// Mod ids that own a negotiated channel or a frozen registry entry.
    pub fn mod_namespaces(&self) -> Vec<String> {
        let mut out = std::collections::BTreeSet::new();
        let channels = self
            .setup
            .iter()
            .flat_map(|s| s.values())
            .flat_map(|c| c.keys());
        let entries = self.frozen.values().flat_map(|s| s.ids.values());
        for id in channels.chain(entries) {
            if let Some((ns, _)) = id.split_once(':')
                && !matches!(
                    ns,
                    "minecraft" | "brigadier" | "neoforge" | "c" | "horizon_bridge"
                )
            {
                out.insert(ns.to_string());
            }
        }
        out.into_iter().collect()
    }

    /// One line per fact, for the client's disconnect screen and the log.
    pub fn summary(&self) -> String {
        let count = |r: &str| self.frozen.get(r).map_or(0, |s| s.ids.len());
        let mods = self.mod_namespaces();
        let mut lines = vec![
            format!("Server: {}", self.brand.as_deref().unwrap_or("unknown")),
            format!(
                "NeoForge negotiation: {}",
                if self.setup.is_some() {
                    "accepted"
                } else {
                    "not offered (vanilla server)"
                }
            ),
            format!("Mods seen: {}", mods.len()),
            format!(
                "Blocks {} · items {} · entity types {}",
                count("minecraft:block"),
                count("minecraft:item"),
                count("minecraft:entity_type")
            ),
        ];
        match &self.block_states {
            Some(s) => lines.push(format!(
                "Block states: {} (from the companion mod)",
                s.state_count()
            )),
            None => lines.push("Block states: not sent (companion mod missing)".into()),
        }
        lines.join("\n")
    }

    pub fn to_json(&self) -> Value {
        let setup = self.setup.as_ref().map(|s| {
            s.iter()
                .map(|(p, c): (&Protocol, _)| (p.name().to_string(), json!(c)))
                .collect::<serde_json::Map<_, _>>()
        });
        let frozen: serde_json::Map<_, _> = self
            .frozen
            .iter()
            .map(|(name, s)| {
                let ids: serde_json::Map<_, _> = s
                    .ids
                    .iter()
                    .map(|(id, n)| (id.to_string(), json!(n)))
                    .collect();
                (name.clone(), json!({ "ids": ids, "aliases": s.aliases }))
            })
            .collect();
        let enums: Vec<Value> = self
            .extensible_enums
            .iter()
            .map(|e| {
                json!({
                    "class": e.class_name,
                    "network_check": e.network_check,
                    "extension": e.extension.as_ref().map(|(v, t, n)| json!({"vanilla": v, "total": t, "added": n})),
                })
            })
            .collect();
        let blocks = self.block_states.as_ref().map(|s| {
            s.blocks
                .iter()
                .map(|b| {
                    json!({
                        "id": b.id,
                        "first_state": b.first_state,
                        "default_state": b.default_state,
                        "properties": b.properties.iter().map(|(n, v)| json!({"name": n, "values": v})).collect::<Vec<_>>(),
                        "states": b.states,
                    })
                })
                .collect::<Vec<_>>()
        });
        json!({
            "format": 1,
            "game_version": crate::GAME_VERSION_767,
            "brand": self.brand,
            "mods": self.mod_namespaces(),
            "channels": setup,
            "listening": self.listening,
            "enabled_features": self.enabled_features,
            "modded_feature_flags": self.modded_feature_flags,
            "extensible_enums": enums,
            "data_maps": self.data_maps,
            "config_files": self.config_files,
            "known_packs": self.known_packs,
            "registries": self.registries,
            "tag_registries": self.tag_registries,
            "frozen_registries": frozen,
            "block_states": blocks,
        })
    }
}
