//! Map a bridge dump's block states onto a block registry and print the stats.
//! `state_map_probe DUMP.json (FACTS.json | blocks.json report)`: a data generator
//! `blocks.json` report stands in for the client's `facts.json`.
use horizon_neoforge_bridge::blocks::StateMap;
use horizon_neoforge_bridge::neoforge::{BlockEntry, BlockStates};
use horizon_world::registry::BlockRegistry;
use serde_json::{Value, json};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let dump: Value = serde_json::from_str(&std::fs::read_to_string(args.next().ok_or("DUMP")?)?)?;
    let client_path = args.next().ok_or("FACTS")?;
    let client: Value = serde_json::from_str(&std::fs::read_to_string(&client_path)?)?;
    let facts = if client.get("states").is_some() {
        client
    } else {
        report_to_facts(&client)
    };
    let registry = BlockRegistry::from_facts_json(facts.to_string().as_bytes())?;
    let blocks = dump["block_states"]
        .as_array()
        .ok_or("dump has no block_states")?;
    let server = BlockStates {
        blocks: blocks
            .iter()
            .map(|b| BlockEntry {
                id: b["id"].as_str().unwrap().into(),
                first_state: b["first_state"].as_i64().unwrap() as i32,
                default_state: b["default_state"].as_i64().unwrap() as i32,
                properties: b["properties"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| {
                        let values = p["values"].as_array().unwrap();
                        (
                            p["name"].as_str().unwrap().into(),
                            values.iter().map(|v| v.as_str().unwrap().into()).collect(),
                        )
                    })
                    .collect(),
                states: b["states"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|s| {
                        s.as_array()
                            .unwrap()
                            .iter()
                            .map(|v| v.as_u64().unwrap() as u8)
                            .collect()
                    })
                    .collect(),
            })
            .collect(),
    };
    let map = StateMap::build(&server, &registry, "minecraft:stone").ok_or("no stone")?;
    println!("{:?}; missing blocks: {:?}", map.stats, map.missing_blocks);
    Ok(())
}

/// A `blocks.json` report as `facts.json` rows (only the state column matters here).
fn report_to_facts(report: &Value) -> Value {
    let mut rows = Vec::new();
    let mut defaults = serde_json::Map::new();
    let mut all: Vec<(i64, &str, &Value)> = Vec::new();
    for (name, block) in report.as_object().unwrap() {
        for state in block["states"].as_array().unwrap() {
            all.push((state["id"].as_i64().unwrap(), name, state));
            if state["default"].as_bool() == Some(true) {
                defaults.insert(name.clone(), state["id"].clone());
            }
        }
    }
    all.sort_by_key(|(id, ..)| *id);
    for (_, name, state) in all {
        let props = state.get("properties").cloned().unwrap_or(json!({}));
        rows.push(json!([name, false, false, "", {"id": name, "properties": props}]));
    }
    json!({ "states": rows, "defaults": defaults })
}
