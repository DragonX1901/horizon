//! Mods' blocks their block entity draws (`RenderShape.ENTITYBLOCK_ANIMATED`, carried over as
//! `animated`): the chunk mesh leaves them out, so their block model is drawn here, turning
//! about the block's axis as fast as its block entity's `Speed` says (the NeoForge bridge's
//! `horizon:block_speeds`). That is Create's KineticBlockEntityRenderer.getAngleForBe:
//! `time * speed * 3 / 10` degrees, time in ticks.
use super::Game;
use horizon_client::mesher::Vertex;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    /// The animated mod blocks among `blocks` (position, state), drawn around `camera` at
    /// `time` ticks.
    pub(super) fn animated_block_geometry(
        &self,
        blocks: impl Iterator<Item = ([i32; 3], u32)>,
        camera: glam::DVec3,
        time: f64,
        out: &mut Vec<Vertex>,
    ) {
        let facts = self.res.facts();
        let registry = &self.pack.assets.registry;
        let speeds = self.remote.as_ref().map(|r| &r.block_speeds);
        let shaft = registry.default_state("create:shaft");
        for (p, state) in blocks {
            let id = BlockStateId(state);
            let f = facts.get(id);
            // Create's other kinetic blocks: its shaft, turning through the casing the chunk drew.
            let shown = if f.kinetic {
                let Some(shaft) = shaft else { continue };
                Some(shaft)
            } else {
                None
            };
            if shown.is_none() && f.render_shape != horizon_client::blocks::RenderShape::Animated
                || f.name.starts_with("minecraft:")
                || self.res.block_entity_render().renders(state)
            {
                continue;
            }
            // A block turns whole only about an axis of its own (shafts, wheels, flywheels); the
            // others (belts, handles) draw still. A kinetic block's shaft always turns.
            let turns = shown.is_some()
                || registry
                    .state(id)
                    .is_some_and(|s| s.properties.contains_key("axis"));
            let speed = speeds
                .and_then(|s| s.get(&p))
                .copied()
                .filter(|_| turns)
                .unwrap_or(0.0);
            let axis = registry.state(id).map_or(glam::Vec3::Y, |s| {
                let named = ["axis", "horizontal_axis", "facing", "horizontal_facing"]
                    .iter()
                    .find_map(|k| s.properties.get(*k))
                    .map(String::as_str);
                match named {
                    Some("x" | "east" | "west") => glam::Vec3::X,
                    Some("z" | "north" | "south") => glam::Vec3::Z,
                    _ => glam::Vec3::Y,
                }
            });
            let angle = ((time * f64::from(speed) * 0.3) % 360.0) as f32;
            let corner = glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
            let half = glam::Vec3::splat(0.5);
            let transform = glam::Mat4::from_translation((corner - camera).as_vec3() + half)
                * glam::Mat4::from_axis_angle(axis, angle.to_radians())
                * glam::Mat4::from_translation(-half);
            let seed = horizon_core::position::BlockPos::new(p[0], p[1], p[2]);
            let drawn = match shown {
                Some(shaft) => {
                    let axis = if axis == glam::Vec3::X {
                        "x"
                    } else if axis == glam::Vec3::Z {
                        "z"
                    } else {
                        "y"
                    };
                    let Some(state) = self.with_props(shaft, &[("axis", axis)]) else {
                        continue;
                    };
                    state
                }
                None => id,
            };
            self.moving_block_vertices(drawn, seed, transform, out);
        }
    }
}
