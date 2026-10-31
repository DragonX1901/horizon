//! Bedrock entity geometry (`*.geo.json`, the format of GeckoLib and Blockbench's
//! Bedrock models) as model parts, for the mobs mods add (`Species::Modded`).
//!
//! Bedrock coordinates are y up from the feet with x mirrored against Java's
//! model space (y down, the feet at 24): a point (x, y, z) is (-x, 24 - y, z).
//! A bone's pivot is absolute; its part's pose is the pivot relative to the
//! parent's, and its cubes are relative to its pivot. Cubes are built as
//! `ModelPart.Cube` builds them: the same corners, faces and box UV layout.
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
        .find(|p| p.exists())?;
    let model = parse(&serde_json::from_slice(&std::fs::read(&geo).ok()?).ok()?)?;
    let texture = [format!("entity/{name}"), format!("entity/{name}/{name}")]
        .into_iter()
        .find(|t| assets.join("textures").join(format!("{t}.png")).exists())
        .map(|t| format!("{ns}:{t}"));
    Some((model, texture))
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

#[cfg(test)]
mod tests {
    use super::*;

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
