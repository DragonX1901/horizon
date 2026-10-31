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
        for (p, state) in blocks {
            let id = BlockStateId(state);
            let f = facts.get(id);
            if f.render_shape != horizon_client::blocks::RenderShape::Animated
                || f.name.starts_with("minecraft:")
                || self.res.block_entity_render().renders(state)
            {
                continue;
            }
            let speed = speeds.and_then(|s| s.get(&p)).copied().unwrap_or(0.0);
            let axis = registry.state(id).map_or(glam::Vec3::Y, |s| {
                let named = s
                    .properties
                    .get("axis")
                    .or_else(|| s.properties.get("facing"))
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
            self.moving_block_vertices(id, seed, transform, out);
        }
    }
}
