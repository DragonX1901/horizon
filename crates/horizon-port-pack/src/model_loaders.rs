//! Block and item models that name a model loader (`"loader"`), which only NeoForge's or the
//! mod's own Java code can read, rewritten as plain models where their parts are plain models:
//!
//! - `neoforge:composite`: its children's elements, one model, each child's texture variables
//!   kept apart (`#base__0`);
//! - `neoforge:separate_transforms`: its `base` model;
//! - `neoforge:obj`: each four-sided face of the mesh as a flat element, turned into place
//!   (26.3's elements take any rotation);
//! - a mod's own loader that names the model it draws (`"model"`, `"base"`): that model.
//!
//! Anything else (models built in code) keeps its loader, and draws nothing.
use serde_json::{Map, Value, json};
use std::fs;
use std::path::{Path, PathBuf};

/// Rewrite the loader models under `inside/assets`; how many.
pub fn flatten(inside: &Path) -> usize {
    let mut files = Vec::new();
    collect(&inside.join("assets"), &mut files);
    let mut count = 0;
    for file in files {
        let Ok(text) = fs::read(&file) else { continue };
        let Ok(mut model) = serde_json::from_slice::<Value>(&text) else {
            continue;
        };
        if model.get("loader").is_none() || !plain(inside, &mut model) {
            continue;
        }
        if let Ok(bytes) = serde_json::to_vec_pretty(&model)
            && fs::write(&file, bytes).is_ok()
        {
            count += 1;
        }
    }
    count
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect(&p, out);
        } else if p.extension().is_some_and(|x| x == "json")
            && p.components().any(|c| c.as_os_str() == "models")
        {
            out.push(p);
        }
    }
}

/// The file of model `reference` (`ns:path`, or a bare path in `minecraft`) under `inside`.
fn file_of(inside: &Path, reference: &str) -> PathBuf {
    let (ns, path) = reference
        .split_once(':')
        .unwrap_or(("minecraft", reference));
    inside.join(format!("assets/{ns}/models/{path}.json"))
}

fn read(inside: &Path, reference: &str) -> Option<Value> {
    serde_json::from_slice(&fs::read(file_of(inside, reference)).ok()?).ok()
}

/// A model's elements and the textures they see, following parents inside the mod.
fn resolve(inside: &Path, model: &Value, depth: usize) -> Option<(Vec<Value>, Map<String, Value>)> {
    let own = model
        .get("textures")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    if let Some(elements) = model.get("elements").and_then(Value::as_array) {
        let mut textures = match model.get("parent").and_then(Value::as_str) {
            Some(p) if depth < 16 => read(inside, p)
                .and_then(|m| resolve(inside, &m, depth + 1))
                .map(|(_, t)| t)
                .unwrap_or_default(),
            _ => Map::new(),
        };
        textures.extend(own);
        return Some((elements.clone(), textures));
    }
    let parent = model
        .get("parent")
        .and_then(Value::as_str)
        .filter(|_| depth < 16)?;
    let (elements, mut textures) = resolve(inside, &read(inside, parent)?, depth + 1)?;
    textures.extend(own);
    Some((elements, textures))
}

/// `#name` → `#{prefix}__name`, in faces and in texture values that point at variables.
fn rename_variable(v: &Value, prefix: &str) -> Value {
    match v.as_str().and_then(|s| s.strip_prefix('#')) {
        Some(var) => json!(format!("#{prefix}__{var}")),
        None => v.clone(),
    }
}

fn plain(inside: &Path, model: &mut Value) -> bool {
    let Some(obj) = model.as_object_mut() else {
        return false;
    };
    let loader = obj
        .get("loader")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    match loader.as_str() {
        "neoforge:composite" => {
            let Some(children) = obj.remove("children").and_then(|c| c.as_object().cloned()) else {
                return false;
            };
            let mut elements = Vec::new();
            let mut textures = obj
                .get("textures")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut unresolved = None;
            for (name, child) in children {
                let child = match child {
                    Value::String(s) => json!({ "parent": s }),
                    c => c,
                };
                let Some((child_elements, child_textures)) = resolve(inside, &child, 0) else {
                    unresolved = child
                        .get("parent")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                    continue;
                };
                for (var, value) in child_textures {
                    textures.insert(format!("{name}__{var}"), rename_variable(&value, &name));
                }
                for mut element in child_elements {
                    for face in element
                        .get_mut("faces")
                        .and_then(Value::as_object_mut)
                        .into_iter()
                        .flat_map(|f| f.values_mut())
                    {
                        if let Some(t) = face.get("texture").map(|t| rename_variable(t, &name)) {
                            face["texture"] = t;
                        }
                    }
                    elements.push(element);
                }
            }
            obj.remove("loader");
            obj.remove("item_render_order");
            obj.remove("visibility");
            obj.insert("textures".into(), Value::Object(textures));
            if elements.is_empty() {
                if let Some(p) = unresolved {
                    obj.insert("parent".into(), json!(p));
                }
            } else {
                obj.insert("elements".into(), Value::Array(elements));
            }
            true
        }
        "neoforge:separate_transforms" => {
            let Some(base) = obj.remove("base") else {
                return false;
            };
            obj.remove("loader");
            obj.remove("perspectives");
            match base {
                Value::String(s) => {
                    obj.insert("parent".into(), json!(s));
                }
                Value::Object(b) => obj.extend(b),
                _ => return false,
            }
            true
        }
        "neoforge:obj" => {
            let Some(reference) = obj.get("model").and_then(Value::as_str) else {
                return false;
            };
            let (ns, path) = reference
                .split_once(':')
                .unwrap_or(("minecraft", reference));
            let file = inside.join(format!("assets/{ns}/{path}"));
            let flip_v = obj.get("flip_v").and_then(Value::as_bool).unwrap_or(false);
            let Some(elements) = fs::read_to_string(&file).ok().and_then(|text| {
                let dir = file.parent()?;
                obj_elements(&text, flip_v, &|mtl| fs::read_to_string(dir.join(mtl)).ok())
            }) else {
                return false;
            };
            for key in [
                "loader",
                "model",
                "flip_v",
                "automatic_culling",
                "shade_quads",
                "emissive_ambient",
                "mtl_override",
            ] {
                obj.remove(key);
            }
            obj.insert("elements".into(), Value::Array(elements));
            true
        }
        l if l.starts_with("neoforge:") => false,
        _ => {
            let named = ["model", "base", "base_model", "parent_model", "inner_model"]
                .iter()
                .filter_map(|k| obj.get(*k).and_then(Value::as_str))
                .find(|r| file_of(inside, r).exists())
                .map(str::to_owned);
            let Some(reference) = named else { return false };
            obj.remove("loader");
            obj.insert("parent".into(), json!(reference));
            true
        }
    }
}

type V3 = [f64; 3];
fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}
fn scale(a: V3, k: f64) -> V3 {
    [a[0] * k, a[1] * k, a[2] * k]
}
fn round(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// An OBJ mesh (block units) as model elements: each quad a flat south face, rotated from
/// its first corner. Its material's `map_Kd` names the texture (`#0`, or a sprite).
fn obj_elements(
    text: &str,
    flip_v: bool,
    mtl: &dyn Fn(&str) -> Option<String>,
) -> Option<Vec<Value>> {
    let mut positions: Vec<V3> = Vec::new();
    let mut uvs: Vec<[f64; 2]> = Vec::new();
    let mut materials: std::collections::HashMap<String, String> = Default::default();
    let mut texture = "#0".to_owned();
    let mut elements = Vec::new();
    for line in text.lines() {
        let mut words = line.split_whitespace();
        let numbers = |w: std::str::SplitWhitespace| {
            w.filter_map(|x| x.parse::<f64>().ok()).collect::<Vec<_>>()
        };
        match words.next() {
            Some("mtllib") => {
                let mut name = None;
                for l in mtl(words.next().unwrap_or_default())
                    .unwrap_or_default()
                    .lines()
                {
                    let mut w = l.split_whitespace();
                    match w.next() {
                        Some("newmtl") => name = w.next().map(str::to_owned),
                        Some("map_Kd") => {
                            if let (Some(n), Some(t)) = (&name, w.next()) {
                                materials.insert(n.clone(), t.to_owned());
                            }
                        }
                        _ => {}
                    }
                }
            }
            Some("usemtl") => {
                let name = words.next().unwrap_or_default();
                texture = materials.get(name).cloned().unwrap_or_else(|| "#0".into());
                if !texture.starts_with('#') && !texture.contains(':') {
                    texture = format!("#{texture}");
                }
            }
            Some("v") => {
                let n = numbers(words);
                positions.push([*n.first()?, *n.get(1)?, *n.get(2)?]);
            }
            Some("vt") => {
                let n = numbers(words);
                let v = *n.get(1)?;
                uvs.push([*n.first()?, if flip_v { 1.0 - v } else { v }]);
            }
            Some("f") => {
                let corners: Vec<(V3, [f64; 2])> = words
                    .filter_map(|c| {
                        let mut parts = c.split('/');
                        let p: usize = parts.next()?.parse().ok()?;
                        let t: usize = parts.next()?.parse().ok()?;
                        Some((
                            scale(*positions.get(p.checked_sub(1)?)?, 16.0),
                            *uvs.get(t.checked_sub(1)?)?,
                        ))
                    })
                    .collect();
                if let Some(e) = quad(&corners, &texture) {
                    elements.push(e);
                }
            }
            _ => {}
        }
    }
    (!elements.is_empty()).then_some(elements)
}

/// One quad (model units, uv 0..1) as a flat element.
fn quad(corners: &[(V3, [f64; 2])], texture: &str) -> Option<Value> {
    if corners.len() != 4 {
        return None;
    }
    // Start at the corner from which the texture's u runs along the first edge.
    let start = (0..4).find(|&i| {
        let (a, b) = (corners[i].1, corners[(i + 1) % 4].1);
        (b[0] - a[0]).abs() > (b[1] - a[1]).abs()
    })?;
    let c = |k: usize| corners[(start + k) % 4];
    let (p0, t0) = c(0);
    let (p1, t1) = c(1);
    let (p3, t3) = c(3);
    let (e1, e2) = (sub(p1, p0), sub(p3, p0));
    let (w, h) = (len(e1), len(e2));
    if w < 1e-6 || h < 1e-6 {
        return None;
    }
    let (u, v) = (scale(e1, 1.0 / w), scale(e2, 1.0 / h));
    let n = cross(u, v);
    // R's columns are u, v, n: local x, y and the south face's normal z, in place.
    // 26.3's Euler rotation is Rz·Ry·Rx: R[2][0] = -sin y.
    let (r00, r10, r20, r21, r22) = (u[0], u[1], u[2], v[2], n[2]);
    let y = (-r20).clamp(-1.0, 1.0).asin();
    let (x, z) = if y.cos().abs() > 1e-6 {
        (r21.atan2(r22), r10.atan2(r00))
    } else {
        ((-n[1]).atan2(v[1]), 0.0)
    };
    let deg = |a: f64| round(a.to_degrees());
    let to = [p0[0] + w, p0[1] + h, p0[2]];
    let uv = [t0[0] * 16.0, t3[1] * 16.0, t1[0] * 16.0, t0[1] * 16.0];
    Some(json!({
        "from": p0.map(round),
        "to": to.map(round),
        "rotation": {"origin": p0.map(round), "x": deg(x), "y": deg(y), "z": deg(z)},
        "faces": {"south": {"uv": uv.map(round), "texture": texture}}
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(root: &Path, reference: &str, v: Value) {
        let f = file_of(root, reference);
        fs::create_dir_all(f.parent().unwrap()).unwrap();
        fs::write(f, v.to_string()).unwrap();
    }

    #[test]
    fn loader_models_become_plain_models() {
        let root = std::env::temp_dir().join(format!("model-loaders-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let cube =
            json!({"from": [0, 0, 0], "to": [16, 16, 16], "faces": {"up": {"texture": "#0"}}});
        put(
            &root,
            "m:block/part",
            json!({"textures": {"0": "m:block/a"}, "elements": [cube]}),
        );
        put(
            &root,
            "m:block/composite",
            json!({
                "loader": "neoforge:composite", "parent": "block/block",
                "children": {
                    "base": {"textures": {"0": "m:block/b"}, "elements": [cube]},
                    "top": {"parent": "m:block/part"}
                }
            }),
        );
        put(
            &root,
            "m:block/jar",
            json!({"loader": "m:jar", "model": "m:block/part"}),
        );
        put(
            &root,
            "m:item/quiver",
            json!({"loader": "neoforge:separate_transforms",
            "base": {"parent": "m:item/in_hand"}, "perspectives": {}}),
        );
        put(
            &root,
            "m:block/mesh",
            json!({"loader": "neoforge:obj", "model": "m:models/x.obj"}),
        );
        put(&root, "m:block/code", json!({"loader": "m:built_in_code"}));
        assert_eq!(flatten(&root), 3);
        let composite = read(&root, "m:block/composite").unwrap();
        assert_eq!(composite["elements"].as_array().unwrap().len(), 2);
        assert_eq!(
            composite["elements"][1]["faces"]["up"]["texture"],
            "#top__0"
        );
        assert_eq!(composite["textures"]["top__0"], "m:block/a");
        assert_eq!(composite["textures"]["base__0"], "m:block/b");
        assert!(composite.get("loader").is_none());
        assert_eq!(
            read(&root, "m:block/jar").unwrap()["parent"],
            "m:block/part"
        );
        assert_eq!(
            read(&root, "m:item/quiver").unwrap()["parent"],
            "m:item/in_hand"
        );
        assert!(read(&root, "m:block/code").unwrap().get("loader").is_some());
        let _ = fs::remove_dir_all(&root);
    }

    /// Where a flat element's corner lands: Rz·Ry·Rx about the origin, as the baker turns it.
    fn place(e: &Value, local: V3) -> V3 {
        let f = |k: &str, i: usize| e[k][i].as_f64().unwrap();
        let r = &e["rotation"];
        let a = |k: &str| r[k].as_f64().unwrap().to_radians();
        let (sx, cx, sy, cy, sz, cz) = (
            a("x").sin(),
            a("x").cos(),
            a("y").sin(),
            a("y").cos(),
            a("z").sin(),
            a("z").cos(),
        );
        let o = [f("from", 0), f("from", 1), f("from", 2)];
        let d = sub(local, o);
        let rows = [
            [cz * cy, -sz * cx + cz * sy * sx, sz * sx + cz * sy * cx],
            [sz * cy, cz * cx + sz * sy * sx, -cz * sx + sz * sy * cx],
            [-sy, cy * sx, cy * cx],
        ];
        std::array::from_fn(|i| o[i] + dot(rows[i], d))
    }

    #[test]
    fn obj_quads_land_where_the_mesh_has_them() {
        let obj = "mtllib a.mtl\nv 0 0 0.5\nv 1 0 0.5\nv 1 1 1\nv 0 1 1\nvt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nusemtl m\nf 1/1 2/2 3/3 4/4\n";
        let elements =
            obj_elements(obj, false, &|_| Some("newmtl m\nmap_Kd #tex\n".into())).unwrap();
        let e = &elements[0];
        assert_eq!(e["faces"]["south"]["texture"], "#tex");
        let (from, to) = (
            [0.0, 0.0, 8.0],
            [
                e["to"][0].as_f64().unwrap(),
                e["to"][1].as_f64().unwrap(),
                8.0,
            ],
        );
        let corner = place(e, [to[0], to[1], 8.0]);
        for (got, want) in corner.iter().zip([16.0, 16.0, 16.0]) {
            assert!((got - want).abs() < 1e-3, "{corner:?}");
        }
        assert_eq!(place(e, from), from);
    }
}
