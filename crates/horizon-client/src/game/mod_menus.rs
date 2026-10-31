//! Mods' menus laid out as their mods lay them out. The NeoForge bridge opens a mod's menu,
//! which the game has no type for, as a chest (`Generic` in play.rs) and names the menu type
//! (`horizon:mod_menu`); the content mods carry each menu's slot positions, recorded by the
//! oracle opening it, and the GUI texture its name points to
//! (`assets/<ns>/horizon/menus.json`, tools/neoforge_bridge/setup_world.py). With them, the
//! chest's slots sit where the mod's do, over the mod's texture.
use super::Game;
use std::collections::HashMap;

/// One mod menu's layout.
#[derive(Clone, Debug, Default)]
pub(super) struct Layout {
    /// The mod's own slots, in menu order.
    pub own: Vec<(f32, f32)>,
    /// The player's slots, by inventory index (0..9 the hotbar).
    pub player: HashMap<usize, (f32, f32)>,
    /// The background (`ns:gui/...`), drawn from its top left corner.
    pub texture: Option<String>,
    pub height: f32,
}

impl Layout {
    fn read(root: &std::path::Path, menu: &str) -> Option<Self> {
        let ns = menu.split(':').next()?;
        let file = root.join(format!("assets/{ns}/horizon/menus.json"));
        let all: serde_json::Value = serde_json::from_slice(&std::fs::read(file).ok()?).ok()?;
        let entry = all.get(menu)?;
        let mut layout = Self {
            texture: entry["texture"].as_str().map(str::to_owned),
            ..Self::default()
        };
        let mut bottom = 0.0f32;
        for slot in entry["slots"].as_array()? {
            let x = slot[0].as_f64()? as f32;
            let y = slot[1].as_f64()? as f32;
            bottom = bottom.max(y);
            match slot[2].as_i64()? {
                i if i >= 0 => {
                    layout.player.insert(i as usize, (x, y));
                }
                _ => layout.own.push((x, y)),
            }
        }
        // A menu's last row is the hotbar, 24 pixels above the bottom edge in vanilla's.
        layout.height = bottom + 24.0;
        Some(layout)
    }
}

impl Game<'_> {
    /// `horizon:mod_menu`: the chest the bridge opens next is this mod menu.
    pub(super) fn remote_mod_menu(&mut self, container: i32, menu: &str) {
        let root = self.options.resources.clone();
        let Some(r) = self.remote.as_mut() else {
            return;
        };
        let layout = r
            .menu_layouts
            .entry(menu.to_owned())
            .or_insert_with(|| Layout::read(&root, menu))
            .clone();
        r.mod_menu = layout.map(|l| (container, l));
    }

    /// The open menu's mod layout, if the bridge named it and the content has it.
    pub(super) fn mod_menu_layout(&self) -> Option<&Layout> {
        let r = self.remote.as_ref()?;
        let (container, layout) = r.mod_menu.as_ref()?;
        (r.menu.as_ref()?.0 == *container).then_some(layout)
    }

    /// Where a slot sits in a mod menu: the mod's slots in order (the chest's padding out of
    /// sight), the player's by inventory index.
    pub(super) fn mod_menu_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        let layout = self.mod_menu_layout()?;
        match i {
            0..36 => layout.player.get(&i).copied(),
            300.. => Some(
                layout
                    .own
                    .get(i - 300)
                    .copied()
                    .unwrap_or((-10_000.0, -10_000.0)),
            ),
            _ => None,
        }
    }

    /// The mod menu's background, title and slot frames; false when the menu has no layout.
    pub(super) fn draw_mod_menu(
        &self,
        batch: &mut horizon_client::hud::HudBatch,
        ox: f32,
        oy: f32,
    ) -> bool {
        use horizon_client::hud::rgba;
        let Some(layout) = self.mod_menu_layout() else {
            return false;
        };
        let atlas = &self.gui.atlas;
        let drawn = layout
            .texture
            .as_deref()
            .is_some_and(|t| atlas.sprites.contains_key(t));
        if let Some(texture) = layout.texture.as_deref().filter(|_| drawn) {
            batch.sprite_region(atlas, texture, [ox, oy, 176.0, layout.height], 256.0, 256.0);
        } else if let Some(r) = atlas.sprites.get("gui/container/generic_54").copied() {
            // No texture of its own: the chest's panel, stretched, with a frame per slot.
            let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
            let part = |x: f32, y: f32, w: f32, h: f32| {
                let mut p = r;
                p.u0 = r.u0 + x * du;
                p.v0 = r.v0 + y * dv;
                p.u1 = p.u0 + w * du;
                p.v1 = p.v0 + h * dv;
                p
            };
            batch.rect_uv(
                ox,
                oy,
                176.0,
                layout.height,
                part(0.0, 0.0, 176.0, 4.0),
                u32::MAX,
            );
            let fill = part(4.0, 4.0, 4.0, 4.0);
            batch.rect_uv(
                ox + 3.0,
                oy + 3.0,
                170.0,
                layout.height - 6.0,
                fill,
                u32::MAX,
            );
            let frame = part(7.0, 17.0, 18.0, 18.0);
            for &(x, y) in layout.own.iter().chain(layout.player.values()) {
                batch.rect_uv(ox + x - 1.0, oy + y - 1.0, 18.0, 18.0, frame, u32::MAX);
            }
        }
        let grey = rgba(64, 64, 64, 255);
        if let Some((_, title)) = self.remote_chest_menu() {
            batch.text_styled(atlas, &title, ox + 8.0, oy + 6.0, grey, false);
        }
        true
    }
}
