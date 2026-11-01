//! Bedrock entity geometry (`*.geo.json`, the format of GeckoLib and Blockbench's
//! Bedrock models) as model parts, for the mobs mods add (`Species::Modded`).
//!
//! Bedrock coordinates are y up from the feet with x mirrored against Java's
//! model space (y down, the feet at 24): a point (x, y, z) is (-x, 24 - y, z).
//! A bone's pivot is absolute; its part's pose is the pivot relative to the
//! parent's, and its cubes are relative to its pivot. Cubes are built as
//! `ModelPart.Cube` builds them: the same corners, faces and box UV layout.
use super::keyframes::{Animation, Channel, Keyframe, Target};
use super::{Part, Polygon};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

/// The model and texture of a mod's mob named `ns:name`, by GeckoLib's usual
/// layout: `assets/ns/geo/[entity/]name.geo.json` and
/// `assets/ns/textures/entity/[name/]name.png`. The texture is a sprite name
/// (`ns:entity/...`), or None when the mod has no texture at those places.
pub fn load(root: &Path, id: &str) -> Option<(Part, Option<String>)> {
    let (ns, name) = id.split_once(':')?;
    let assets = root.join("assets").join(ns);
    let geo = ["geo", "geo/entity", "geo/entities", "geo/mob"]
        .iter()
        .map(|dir| assets.join(dir).join(format!("{name}.geo.json")))
        .find(|p| p.exists())
        // Its first numbered variant (Mowzie's geo/boulder_1.geo.json for `boulder`).
        .or_else(|| {
            let mut found: Vec<_> = ["geo", "geo/entity", "geo/entities", "geo/mob"]
                .iter()
                .flat_map(|dir| {
                    std::fs::read_dir(assets.join(dir))
                        .into_iter()
                        .flatten()
                        .flatten()
                })
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                        n.strip_prefix(&format!("{name}_"))
                            .and_then(|r| r.strip_suffix(".geo.json"))
                            .is_some_and(|r| r.chars().all(|c| c.is_ascii_digit()) && !r.is_empty())
                    })
                })
                .collect();
            found.sort();
            found.into_iter().next()
        });
    let model = match geo {
        Some(geo) => parse(&serde_json::from_slice(&std::fs::read(&geo).ok()?).ok()?)?,
        // Models the mod builds in Java, baked by tools/neoforge_bridge/export_mod_models.py.
        None => {
            let baked: Value = serde_json::from_slice(
                &std::fs::read(assets.join("horizon/entity_models.json")).ok()?,
            )
            .ok()?;
            let entry = baked.get(id)?;
            // A mob drawn with a vanilla model: not a part tree (see `vanilla_alias`).
            if entry.get("vanilla").is_some() {
                return None;
            }
            Part::parse("root", entry)
        }
    };
    Some((model, texture(root, id)))
}

/// The texture of a mod's mob `ns:name`: at GeckoLib's usual places
/// (`assets/ns/textures/entity/[name/]name.png`), else a `name.png` anywhere under
/// `textures/entity` (Aether's `entity/mobs/flying_cow/flying_cow.png`), as a sprite name.
pub fn texture(root: &Path, id: &str) -> Option<String> {
    let (ns, name) = id.split_once(':')?;
    let textures = root.join("assets").join(ns).join("textures");
    if let Some(t) = [format!("entity/{name}"), format!("entity/{name}/{name}")]
        .into_iter()
        .find(|t| textures.join(format!("{t}.png")).exists())
    {
        return Some(format!("{ns}:{t}"));
    }
    let sprite = |p: &Path| -> Option<String> {
        let rest = p.strip_prefix(&textures).ok()?.with_extension("");
        Some(format!(
            "{ns}:{}",
            rest.to_string_lossy().replace('\\', "/")
        ))
    };
    // Entity folders: textures/entity, and one level in (Quark's textures/model/entity).
    let mut roots = vec![textures.join("entity")];
    for d in std::fs::read_dir(&textures).into_iter().flatten().flatten() {
        let nested = d.path().join("entity");
        if d.file_name() != "entity" && nested.is_dir() {
            roots.push(nested);
        }
    }
    let pngs = |dir: &Path| {
        let mut out = Vec::new();
        let mut stack = vec![dir.to_owned()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "png") {
                    out.push(p);
                }
            }
        }
        out.sort();
        out
    };
    // `name.png` anywhere in them (Aether's entity/mobs/flying_cow/flying_cow.png).
    for root in &roots {
        if let Some(p) = pngs(root)
            .into_iter()
            .find(|p| p.file_stem().is_some_and(|s| s == name))
        {
            return sprite(&p);
        }
    }
    // Else the first plain texture in a folder named after it (variants: Quark's
    // foxhound/red/...), not an overlay.
    const OVERLAYS: [&str; 7] = [
        "armor", "collar", "glow", "eyes", "overlay", "saddle", "emissive",
    ];
    for root in &roots {
        let dir = root.join(name);
        if let Some(p) = pngs(&dir).into_iter().find(|p| {
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            !OVERLAYS.iter().any(|o| stem.contains(o))
        }) {
            return sprite(&p);
        }
    }
    None
}

/// The vanilla mob a mod's mob is drawn as when its renderer uses a vanilla model
/// (`{"vanilla": "minecraft:cow"}` in `entity_models.json`, export_mod_models.py).
pub fn vanilla_alias(root: &Path, id: &str) -> Option<String> {
    let (ns, _) = id.split_once(':')?;
    let file = root
        .join("assets")
        .join(ns)
        .join("horizon/entity_models.json");
    let baked: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    baked.get(id)?.get("vanilla")?.as_str().map(str::to_owned)
}

/// The first geometry of a `.geo.json` (format 1.12 and later) as a root part.
pub fn parse(doc: &Value) -> Option<Part> {
    let geometry = doc["minecraft:geometry"].as_array()?.first()?;
    let desc = &geometry["description"];
    let tex = [
        desc["texture_width"].as_f64().unwrap_or(64.0) as f32,
        desc["texture_height"].as_f64().unwrap_or(64.0) as f32,
    ];
    let bones = geometry["bones"].as_array()?;
    let f3 =
        |v: &Value| -> [f32; 3] { std::array::from_fn(|i| v[i].as_f64().unwrap_or(0.0) as f32) };
    // Each bone's absolute pivot in Java model space.
    let pivot = |b: &Value| {
        let p = f3(&b["pivot"]);
        [-p[0], 24.0 - p[1], p[2]]
    };
    let by_name: HashMap<&str, &Value> = bones
        .iter()
        .filter_map(|b| Some((b["name"].as_str()?, b)))
        .collect();
    let mut children: HashMap<Option<&str>, Vec<&Value>> = HashMap::new();
    for b in bones {
        let parent = b["parent"].as_str().filter(|p| by_name.contains_key(p));
        children.entry(parent).or_default().push(b);
    }
    fn build<'a>(
        bone: &'a Value,
        parent_pivot: [f32; 3],
        children: &HashMap<Option<&'a str>, Vec<&'a Value>>,
        pivot: &dyn Fn(&Value) -> [f32; 3],
        tex: [f32; 2],
    ) -> Part {
        let at = pivot(bone);
        let r = &bone["rotation"];
        let deg = |i: usize| r[i].as_f64().unwrap_or(0.0) as f32;
        // GeckoLib: x and y rotations flip with the mirrored axes.
        let pose = [
            at[0] - parent_pivot[0],
            at[1] - parent_pivot[1],
            at[2] - parent_pivot[2],
            (-deg(0)).to_radians(),
            (-deg(1)).to_radians(),
            deg(2).to_radians(),
            1.0,
            1.0,
            1.0,
        ];
        let mut polygons = Vec::new();
        for cube in bone["cubes"].as_array().into_iter().flatten() {
            polygons.extend(cube_polygons(
                cube,
                at,
                tex,
                bone["mirror"].as_bool().unwrap_or(false),
            ));
        }
        let name = bone["name"].as_str().unwrap_or("bone").to_owned();
        let kids = children
            .get(&Some(bone["name"].as_str().unwrap_or("")))
            .into_iter()
            .flatten()
            .map(|c| build(c, at, children, pivot, tex))
            .collect();
        Part {
            name,
            pose,
            polygons,
            children: kids,
        }
    }
    let roots = children.get(&None)?;
    Some(Part {
        name: "root".into(),
        pose: [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 1.0],
        polygons: Vec::new(),
        children: roots
            .iter()
            .map(|b| build(b, [0.0; 3], &children, &pivot, tex))
            .collect(),
    })
}

/// A Bedrock cube's faces, relative to its bone's pivot `at` (Java space).
fn cube_polygons(cube: &Value, at: [f32; 3], tex: [f32; 2], bone_mirror: bool) -> Vec<Polygon> {
    let f3 =
        |v: &Value| -> [f32; 3] { std::array::from_fn(|i| v[i].as_f64().unwrap_or(0.0) as f32) };
    let (origin, size) = (f3(&cube["origin"]), f3(&cube["size"]));
    let grow = cube["inflate"].as_f64().unwrap_or(0.0) as f32;
    let mirror = cube["mirror"].as_bool().unwrap_or(bone_mirror);
    // The cube's minimum corner in Java space, relative to the pivot.
    let from = [
        -(origin[0] + size[0]) - at[0],
        24.0 - (origin[1] + size[1]) - at[1],
        origin[2] - at[2],
    ];
    let (mut x0, y0, z0) = (from[0] - grow, from[1] - grow, from[2] - grow);
    let (mut x1, y1, z1) = (
        from[0] + size[0] + grow,
        from[1] + size[1] + grow,
        from[2] + size[2] + grow,
    );
    if mirror {
        std::mem::swap(&mut x0, &mut x1);
    }
    let v = [
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y1, z0],
        [x0, y1, z0],
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
    ];
    // Each face: its corners and its UV rectangle (u1, v1, u2, v2) in texels.
    let (w, h, d) = (size[0].floor(), size[1].floor(), size[2].floor());
    let faces: Vec<([usize; 4], [f32; 4], &str)> = match &cube["uv"] {
        Value::Array(uv) => {
            let (u, vv) = (
                uv[0].as_f64().unwrap_or(0.0) as f32,
                uv[1].as_f64().unwrap_or(0.0) as f32,
            );
            let (u0, u1, u2, u3, u4, u5) = (
                u,
                u + d,
                u + d + w,
                u + d + w + w,
                u + d + w + d,
                u + d + w + d + w,
            );
            let (v0, v1, v2) = (vv, vv + d, vv + d + h);
            vec![
                ([5, 4, 0, 1], [u1, v0, u2, v1], "down"),
                ([2, 3, 7, 6], [u2, v1, u3, v0], "up"),
                ([0, 4, 7, 3], [u0, v1, u1, v2], "west"),
                ([1, 0, 3, 2], [u1, v1, u2, v2], "north"),
                ([5, 1, 2, 6], [u2, v1, u4, v2], "east"),
                ([4, 5, 6, 7], [u4, v1, u5, v2], "south"),
            ]
        }
        Value::Object(per_face) => {
            let corners: [([usize; 4], &str); 6] = [
                ([5, 4, 0, 1], "down"),
                ([2, 3, 7, 6], "up"),
                ([0, 4, 7, 3], "west"),
                ([1, 0, 3, 2], "north"),
                ([5, 1, 2, 6], "east"),
                ([4, 5, 6, 7], "south"),
            ];
            corners
                .into_iter()
                .filter_map(|(c, face)| {
                    // Bedrock's west and east are Java's east and west (x mirrored).
                    let key = match face {
                        "west" => "east",
                        "east" => "west",
                        f => f,
                    };
                    let f = per_face.get(key)?;
                    let uv = f3(&f["uv"]);
                    let sz = f3(&f["uv_size"]);
                    Some((c, [uv[0], uv[1], uv[0] + sz[0], uv[1] + sz[1]], face))
                })
                .collect()
        }
        _ => Vec::new(),
    };
    faces
        .into_iter()
        .map(|(c, [u1, v1, u2, v2], _)| {
            let uv = [[u2, v1], [u1, v1], [u1, v2], [u2, v2]];
            let mut vertices: [[f32; 5]; 4] = std::array::from_fn(|i| {
                let p = v[c[i]];
                [p[0], p[1], p[2], uv[i][0] / tex[0], uv[i][1] / tex[1]]
            });
            if mirror {
                vertices.reverse();
            }
            let p = vertices.map(|q| glam::Vec3::new(q[0], q[1], q[2]));
            let normal = (p[1] - p[0])
                .cross(p[2] - p[0])
                .normalize_or_zero()
                .to_array();
            Polygon { normal, vertices }
        })
        .collect()
}

/// A mod mob's idle and walk animations (GeckoLib's `.animation.json`).
#[derive(Clone, Debug, Default)]
pub struct Animations {
    pub idle: Option<Animation>,
    pub walk: Option<Animation>,
}

/// The animations of the mob `ns:name`: `assets/ns/animations/[entity/]name.animation.json`,
/// the first whose name has `idle` and the first with `walk` or `move`.
pub fn load_animations(root: &Path, id: &str) -> Option<Animations> {
    let (ns, name) = id.split_once(':')?;
    let assets = root.join("assets").join(ns);
    let file = [
        "animations",
        "animations/entity",
        "animations/entities",
        "animations/mob",
    ]
    .iter()
    .map(|dir| assets.join(dir).join(format!("{name}.animation.json")))
    .find(|p| p.exists())?;
    animations_from(&file)
}

/// A `.animation.json`'s idle and walk: on-ground ones first (Cobblemon's
/// `ground_idle`, `ground_walk`), else any idle and any walk or move.
fn animations_from(file: &Path) -> Option<Animations> {
    let doc: Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
    let all = doc["animations"].as_object()?;
    let find = |words: &[&str]| {
        words.iter().find_map(|w| {
            all.iter()
                .find(|(k, _)| k.to_lowercase().contains(w))
                .map(|(_, v)| animation(v))
        })
    };
    Some(Animations {
        idle: find(&["ground_idle", "idle"]),
        walk: find(&["ground_walk", "walk", "move"]),
    })
}

/// The look of a mob whose entity data names a species with Cobblemon's
/// resolver files (`assets/ns/bedrock/pokemon/resolvers/<n>_<name>/*.json`):
/// its base variation's model and texture, and the species' animations.
pub fn species_look(root: &Path, species: &str) -> Option<super::ModdedLook> {
    let (ns, name) = species.split_once(':')?;
    let bedrock = root.join("assets").join(ns).join("bedrock/pokemon");
    let folder = std::fs::read_dir(bedrock.join("resolvers"))
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .find(|f| f.split_once('_').is_some_and(|(_, n)| n == name))?;
    let mut resolvers: Vec<Value> = std::fs::read_dir(bedrock.join("resolvers").join(&folder))
        .ok()?
        .flatten()
        .filter_map(|e| serde_json::from_slice(&std::fs::read(e.path()).ok()?).ok())
        .filter(|v: &Value| v["species"].as_str() == Some(species))
        .collect();
    resolvers.sort_by_key(|v| v["order"].as_i64().unwrap_or(0));
    let base = resolvers
        .iter()
        .flat_map(|r| r["variations"].as_array().into_iter().flatten())
        .find(|v| v["aspects"].as_array().is_none_or(|a| a.is_empty()) && v["model"].is_string())?;
    let model_file = base["model"]
        .as_str()?
        .split_once(':')?
        .1
        .trim_end_matches(".geo");
    let geo = bedrock
        .join("models")
        .join(&folder)
        .join(format!("{model_file}.geo.json"));
    let model = parse(&serde_json::from_slice(&std::fs::read(geo).ok()?).ok()?)?;
    // cobblemon:textures/pokemon/0014_kakuna/kakuna.png -> cobblemon:pokemon/0014_kakuna/kakuna
    let texture = base["texture"].as_str().and_then(|t| {
        let (tns, path) = t.split_once(':')?;
        Some(format!(
            "{tns}:{}",
            path.strip_prefix("textures/")?.strip_suffix(".png")?
        ))
    })?;
    let animations = animations_from(
        &bedrock
            .join("animations")
            .join(&folder)
            .join(format!("{model_file}.animation.json")),
    );
    Some(super::ModdedLook {
        model: std::sync::Arc::new(model),
        texture,
        animations: animations.map(std::sync::Arc::new),
        scale: species_scale(root, ns, name).unwrap_or(1.0),
    })
}

/// A species' `baseScale` from its data file (`data/ns/species/**/name.json`).
fn species_scale(root: &Path, ns: &str, name: &str) -> Option<f32> {
    let mut stack = vec![root.join("data").join(ns).join("species")];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).ok()?.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p
                .file_name()
                .is_some_and(|f| f == format!("{name}.json").as_str())
            {
                let v: Value = serde_json::from_slice(&std::fs::read(&p).ok()?).ok()?;
                return v["baseScale"].as_f64().map(|s| s as f32);
            }
        }
    }
    None
}

/// One Bedrock animation as a keyframe animation in Java model space: rotations
/// in radians with x and y flipped (as the bones'), positions with x and y
/// flipped, scales as offsets from 1. Molang expressions count as 0.
fn animation(v: &Value) -> Animation {
    let num = |v: &Value| v.as_f64().unwrap_or(0.0) as f32;
    let vec = |v: &Value| -> Option<[f32; 3]> {
        match v {
            Value::Array(a) => Some(std::array::from_fn(|i| {
                num(&a[i.min(a.len().saturating_sub(1))])
            })),
            Value::Number(n) => Some([n.as_f64().unwrap_or(0.0) as f32; 3]),
            Value::Object(o) => o
                .get("vector")
                .or_else(|| o.get("post"))
                .and_then(|x| match x {
                    Value::Array(a) => Some(std::array::from_fn(|i| {
                        num(&a[i.min(a.len().saturating_sub(1))])
                    })),
                    _ => None,
                }),
            _ => None,
        }
    };
    let convert = |target: Target, p: [f32; 3]| -> [f32; 3] {
        match target {
            Target::Rotation => [
                (-p[0]).to_radians(),
                (-p[1]).to_radians(),
                p[2].to_radians(),
            ],
            Target::Position => [-p[0], -p[1], p[2]],
            Target::Scale => [p[0] - 1.0, p[1] - 1.0, p[2] - 1.0],
        }
    };
    let mut bones = Vec::new();
    for (bone, channels) in v["bones"].as_object().into_iter().flatten() {
        let mut out = Vec::new();
        for (key, target) in [
            ("rotation", Target::Rotation),
            ("position", Target::Position),
            ("scale", Target::Scale),
        ] {
            let c = &channels[key];
            let mut keyframes: Vec<Keyframe> = match c {
                Value::Object(times)
                    if !times.contains_key("vector") && !times.contains_key("post") =>
                {
                    times
                        .iter()
                        .filter_map(|(t, val)| {
                            let p = convert(target, vec(val)?);
                            let catmull = val["lerp_mode"].as_str() == Some("catmullrom");
                            Some(Keyframe {
                                time: t.parse().ok()?,
                                pre: p,
                                post: p,
                                catmull,
                            })
                        })
                        .collect()
                }
                Value::Null => continue,
                other => match vec(other) {
                    Some(p) => {
                        let p = convert(target, p);
                        vec![Keyframe {
                            time: 0.0,
                            pre: p,
                            post: p,
                            catmull: false,
                        }]
                    }
                    None => continue,
                },
            };
            keyframes.sort_by(|a, b| a.time.total_cmp(&b.time));
            if !keyframes.is_empty() {
                out.push(Channel { target, keyframes });
            }
        }
        if !out.is_empty() {
            bones.push((bone.clone(), out));
        }
    }
    Animation {
        length: v["animation_length"]
            .as_f64()
            .map_or(1.0, |l| l as f32)
            .max(0.05),
        looping: v["loop"].as_bool().unwrap_or(false) || v["loop"].as_str() == Some("true"),
        bones,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_animation() {
        let doc: Value = serde_json::from_str(
            r#"{"loop":true,"animation_length":2.0,"bones":{"head":{
                "rotation":{"0.0":[0,0,0],"1.0":[10,0,0],"2.0":[0,0,0]},
                "position":[0,1,0]}}}"#,
        )
        .unwrap();
        let a = animation(&doc);
        assert!(a.looping);
        assert_eq!(a.length, 2.0);
        let (bone, channels) = &a.bones[0];
        assert_eq!(bone, "head");
        let rot = channels
            .iter()
            .find(|c| c.target == Target::Rotation)
            .unwrap();
        assert_eq!(rot.keyframes.len(), 3);
        assert!((rot.keyframes[1].post[0] + 10f32.to_radians()).abs() < 1e-6);
        let pos = channels
            .iter()
            .find(|c| c.target == Target::Position)
            .unwrap();
        assert_eq!(pos.keyframes[0].post, [0.0, -1.0, 0.0]);
    }

    #[test]
    fn a_bone_with_a_cube() {
        let doc: Value = serde_json::from_str(
            r#"{"format_version":"1.12.0","minecraft:geometry":[{
                "description":{"identifier":"geometry.box","texture_width":64,"texture_height":32},
                "bones":[
                    {"name":"body","pivot":[0,12,0],"cubes":[{"origin":[-4,6,-2],"size":[8,12,4],"uv":[16,16]}]},
                    {"name":"head","parent":"body","pivot":[0,24,0],"rotation":[10,0,0],
                     "cubes":[{"origin":[-4,24,-4],"size":[8,8,8],"uv":[0,0]}]}
                ]}]}"#,
        )
        .unwrap();
        let root = parse(&doc).unwrap();
        let body = &root.children[0];
        assert_eq!(body.name, "body");
        assert_eq!(&body.pose[..3], &[0.0, 12.0, 0.0]);
        assert_eq!(body.polygons.len(), 6);
        // The body cube spans y 6..18 (Bedrock): 6..18 below the feet line, from the pivot at 12.
        let ys: Vec<f32> = body
            .polygons
            .iter()
            .flat_map(|p| p.vertices.iter().map(|v| v[1]))
            .collect();
        let (min, max) = ys
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), &y| (a.min(y), b.max(y)));
        assert_eq!((min, max), (-6.0, 6.0));
        let head = &body.children[0];
        assert_eq!(&head.pose[..3], &[0.0, -12.0, 0.0]);
        assert!((head.pose[3] + 10f32.to_radians()).abs() < 1e-6);
        // Box UV: the north face of the 8x12x4 body at (16, 16) is u 20..28, v 20..32.
        let north = &body.polygons[3];
        let us: Vec<f32> = north.vertices.iter().map(|v| v[3] * 64.0).collect();
        assert!(us.contains(&20.0) && us.contains(&28.0));
    }
}
