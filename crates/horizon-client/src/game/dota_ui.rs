use super::Game;
use horizon_client::hud::HudBatch;
use horizon_dota::{data::find_data_dir, panorama_hud};
use panorama::Doc;

pub(super) struct DotaUi {
    doc: Doc,
}

impl Game<'_> {
    fn dota_ui_load(&self) {
        if self.dota_ui.borrow().is_some() || self.dota_ui_failed.get() {
            return;
        }
        let Some(dir) = find_data_dir() else {
            self.dota_ui_failed.set(true);
            return;
        };
        let root = dir.join("panorama").join("panorama");
        match Doc::open(&root, "layout/hud/hud_reborn.xml") {
            Some(mut doc) => {
                panorama_hud::prepare(&mut doc);
                *self.dota_ui.borrow_mut() = Some(DotaUi { doc });
            }
            None => {
                log::warn!("Dota: the Panorama HUD did not load from {}", root.display());
                self.dota_ui_failed.set(true);
            }
        }
    }

    pub(super) fn dota_ui_paint(&self, batch: &HudBatch) -> Option<icraft_ui::Paint> {
        self.dota_ui_load();
        let s = self.dota.clone()?;
        let mut ui = self.dota_ui.borrow_mut().take()?;
        let (w, h) = (batch.width, batch.height);
        let k = h / panorama::REFERENCE_HEIGHT;
        let scale = batch.scale;
        let name = self.dota_data.as_ref().map(|d| d.display_name(&s.hero)).unwrap_or_default();
        let dots = panorama_hud::minimap_dots(&s);
        panorama_hud::bind(&mut ui.doc, &s, &name, &dots);
        let atlas = &self.gui.atlas;
        let measure = |text: &str, font: &icraft_ui::Font| {
            horizon_client::dota_text::measure(atlas, text, &font.family, font.weight, font.size * k, scale) / k
        };
        let resolve = |url: &str| -> Option<(String, f32, f32)> {
            let name = if let Some(rest) = url.strip_prefix("dota:") {
                format!("dota/{rest}")
            } else {
                format!("dota/p/{}", panorama::image_key(url)?.to_lowercase())
            };
            let r = atlas.sprites.get(&format!("gui/{name}"))?;
            Some((name, r.w as f32, r.h as f32))
        };
        let paint = ui.doc.render(w, h, &measure, &resolve);
        *self.dota_ui.borrow_mut() = Some(ui);
        Some(paint)
    }
}
