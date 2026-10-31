use super::{Game, Screen};
use horizon_client::hud::{HudBatch, rgba};
use horizon_dota::{
    data::{Data, find_data_dir},
    hud::ShopState,
    sim::{Aim, Intent},
    wire,
};
use horizon_protocol::packets::{configuration::CustomPayload, play};
use std::sync::Arc;
use winit::{event::MouseButton, keyboard::KeyCode};

const ABILITY_KEYS: [(KeyCode, &str); 6] = [
    (KeyCode::KeyQ, "Q"),
    (KeyCode::KeyE, "E"),
    (KeyCode::KeyR, "R"),
    (KeyCode::KeyF, "F"),
    (KeyCode::KeyG, "G"),
    (KeyCode::KeyH, "H"),
];

const ITEM_KEYS: [KeyCode; 6] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
];

impl Game<'_> {
    fn dota_ensure_data(&mut self) -> bool {
        if self.dota_data.is_none() {
            self.dota_data = find_data_dir().and_then(|dir| Data::load(&dir).ok()).map(Arc::new);
        }
        self.dota_data.is_some()
    }

    pub(super) fn dota_receive(&mut self, data: &[u8]) {
        match wire::decode_message(data) {
            Ok(wire::Message::Pick { taken }) => {
                if self.dota_ensure_data() {
                    let state = self.dota_pick.take().map(|(_, s)| s).unwrap_or_default();
                    self.dota_pick = Some((taken, state));
                    self.capture(false);
                }
            }
            Ok(wire::Message::Sound { event, pos }) => {
                let id = format!("dota:{}", event.to_ascii_lowercase());
                self.audio.play(&id, Some([f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2])]), 1.0, 1.0);
            }
            Ok(wire::Message::Snapshot(snapshot)) => {
                let snapshot = *snapshot;
                if self.dota_pick.take().is_some() {
                    self.capture(true);
                }
                let hold = self.left && (self.captured || self.control.is_some()) && self.dota_shop.is_none() && snapshot.alive;
                self.dota = Some(snapshot);
                if hold {
                    let look = self.forward();
                    self.dota_send(&Intent::Attack {
                        dir: [look[0] as f32, look[1] as f32, look[2] as f32],
                    });
                }
            }
            Err(e) => log::warn!("Dota: bad message: {e}"),
        }
    }

    fn dota_send(&mut self, intent: &Intent) {
        self.send(play::Serverbound::CustomPayload(CustomPayload {
            channel: wire::CHANNEL.into(),
            data: wire::encode_intent(intent),
        }));
    }

    fn dota_gui_scale(&self) -> f32 {
        self.renderer.as_ref().map_or(1.0, |r| HudBatch::new([r.config.width, r.config.height]).scale)
    }

    fn dota_open_shop(&mut self) {
        if !self.dota_ensure_data() {
            log::warn!("Dota: no data folder for the shop");
            return;
        }
        self.dota_shop = Some(ShopState::default());
        self.capture(false);
    }

    fn dota_close_shop(&mut self) {
        self.dota_shop = None;
        self.capture(true);
    }

    pub(super) fn dota_key(&mut self, key: KeyCode) -> bool {
        if self.dota.is_none() || self.screen != Screen::Playing || self.remote.is_none() {
            return false;
        }
        if key == KeyCode::KeyB {
            if self.dota_shop.is_some() {
                self.dota_close_shop();
            } else {
                self.dota_open_shop();
            }
            return true;
        }
        if key == KeyCode::Escape && self.dota_shop.is_some() {
            self.dota_close_shop();
            return true;
        }
        let courier = match key {
            KeyCode::KeyC => Some(horizon_dota::courier::CourierCommand::Deliver),
            KeyCode::KeyV => Some(horizon_dota::courier::CourierCommand::Burst),
            KeyCode::KeyX => Some(horizon_dota::courier::CourierCommand::Home),
            _ => None,
        };
        if let Some(command) = courier {
            self.dota_send(&Intent::Courier(command));
            return true;
        }
        let ctrl = self.modifiers.control_key();
        if let Some(slot) = ABILITY_KEYS.iter().position(|(k, _)| *k == key) {
            let learnable = self.dota.as_ref().and_then(|s| s.abilities.get(slot)).is_some_and(|a| a.level == 0 && a.can_level);
            let intent = if ctrl || learnable {
                Intent::LevelUp(slot)
            } else {
                let look = self.forward();
                Intent::Cast {
                    slot,
                    aim: Aim::Direction([look[0] as f32, look[1] as f32, look[2] as f32]),
                }
            };
            self.dota_send(&intent);
            return true;
        }
        if let Some(slot) = ITEM_KEYS.iter().position(|k| *k == key) {
            let look = self.forward();
            self.dota_send(&Intent::UseItem {
                slot,
                aim: Aim::Direction([look[0] as f32, look[1] as f32, look[2] as f32]),
            });
            return true;
        }
        false
    }

    fn dota_pick_paint(&self, w: f32, h: f32, mouse: (f32, f32), hover: Option<String>) -> Option<icraft_ui::Paint> {
        let (taken, state) = self.dota_pick.as_ref()?;
        let data = self.dota_data.as_ref()?;
        let state = horizon_dota::hud::PickState {
            hover,
            ..state.clone()
        };
        let atlas = &self.gui.atlas;
        let has_icon = |src: &str| atlas.sprites.contains_key(&format!("gui/{src}"));
        let (html, css) = horizon_dota::hud::pick_document(data, taken, &state, w, h, &has_icon);
        Some(self.paint_rich(&html, &css, w, h, mouse, self.dota_gui_scale()))
    }

    pub(super) fn dota_mouse(&mut self, button: MouseButton, pressed: bool) -> bool {
        if self.dota_pick.is_some() {
            if pressed && button == MouseButton::Left {
                let Some(r) = &self.renderer else { return true };
                let batch = HudBatch::new([r.config.width, r.config.height]);
                let action = self
                    .dota_pick_paint(batch.width, batch.height, self.cursor, None)
                    .and_then(|p| icraft_ui::click_at(&p, self.cursor.0, self.cursor.1).map(str::to_owned));
                match action.as_deref().and_then(|a| a.split_once(':')) {
                    Some(("pick", name)) => {
                        if let Some((_, state)) = self.dota_pick.as_mut() {
                            state.selected = Some(name.to_owned());
                        }
                    }
                    _ if action.as_deref() == Some("lock") => {
                        let chosen = self.dota_pick.as_ref().and_then(|(_, s)| s.selected.clone());
                        if let Some(name) = chosen {
                            self.dota_send(&Intent::PickHero(name));
                        }
                    }
                    _ => {}
                }
            }
            return true;
        }
        if self.dota.is_none() {
            return false;
        }
        if self.dota_shop.is_none() {
            if pressed && button == MouseButton::Left && self.captured && self.screen == Screen::Playing {
                let look = self.forward();
                self.dota_send(&Intent::Attack {
                    dir: [look[0] as f32, look[1] as f32, look[2] as f32],
                });
            }
            return false;
        }
        if !pressed || button != MouseButton::Left {
            return true;
        }
        let Some(r) = &self.renderer else { return true };
        let batch = HudBatch::new([r.config.width, r.config.height]);
        let Some(paint) = self.dota_shop_paint(batch.width, batch.height, self.cursor, None) else {
            return true;
        };
        let Some(action) = icraft_ui::click_at(&paint, self.cursor.0, self.cursor.1).map(str::to_owned) else {
            return true;
        };
        match action.split_once(':') {
            Some(("tab", n)) => {
                if let (Some(shop), Ok(n)) = (self.dota_shop.as_mut(), n.parse()) {
                    shop.tab = n;
                }
            }
            Some(("buy", item)) => self.dota_send(&Intent::Buy {
                item: item.to_owned(),
                at_shop: false,
            }),
            Some(("sell", slot)) => {
                if let Ok(slot) = slot.parse() {
                    self.dota_send(&Intent::Sell { slot, at_shop: false });
                }
            }
            _ => {}
        }
        true
    }

    fn paint_rich(&self, html: &str, css: &str, w: f32, h: f32, mouse: (f32, f32), scale: f32) -> icraft_ui::Paint {
        let atlas = &self.gui.atlas;
        let legacy = |text: &str, bold: bool| HudBatch::text_width(atlas, text) + if bold { text.chars().count() as f32 } else { 0.0 };
        let rich = |text: &str, font: &icraft_ui::Font| {
            horizon_client::dota_text::measure(atlas, text, &font.family, font.weight, font.size, scale)
        };
        icraft_ui::paint_rich(html, css, w, h, mouse.0, mouse.1, &legacy, Some(&rich))
    }

    fn dota_shop_paint(&self, w: f32, h: f32, mouse: (f32, f32), hover: Option<String>) -> Option<icraft_ui::Paint> {
        let s = self.dota.as_ref()?;
        let data = self.dota_data.as_ref()?;
        let state = ShopState {
            hover,
            ..self.dota_shop.clone()?
        };
        let atlas = &self.gui.atlas;
        let has_icon = |src: &str| atlas.sprites.contains_key(&format!("gui/{src}"));
        let (html, css) = horizon_dota::hud::shop_document(s, data, &state, w, h, &has_icon);
        Some(self.paint_rich(&html, &css, w, h, mouse, self.dota_gui_scale()))
    }

    fn draw_paint(&self, batch: &mut HudBatch, paint: &icraft_ui::Paint) {
        let atlas = &self.gui.atlas;
        for op in &paint.ops {
            match op {
                icraft_ui::Op::Fill(f) => batch.fill(atlas, f.x, f.y, f.w, f.h, hud_color(f.color)),
                icraft_ui::Op::Gradient(g) => {
                    batch.fill_gradient(atlas, (g.x, g.y, g.w, g.h), hud_color(g.top), hud_color(g.bottom));
                }
                icraft_ui::Op::Image(i) => {
                    let tint = rgba(255, 255, 255, (i.opacity.clamp(0.0, 1.0) * 255.0) as u8);
                    batch.sprite_scaled(atlas, &format!("gui/{}", i.src), i.x, i.y, i.w, i.h, tint);
                }
                icraft_ui::Op::Label(l) if !l.family.is_empty() => {
                    let shadow = l.shadow.map(hud_color);
                    horizon_client::dota_text::draw(atlas, batch, &l.text, &l.family, l.weight, l.size, l.line_height, l.x, l.y, hud_color(l.color), shadow);
                }
                icraft_ui::Op::Label(l) => {
                    let start = batch.vertices.len();
                    batch.text(atlas, &l.text, l.x, l.y, hud_color(l.color), true);
                    if (l.scale - 1.0).abs() > 0.01 {
                        batch.scale_since(start, l.x, l.y, l.scale, l.scale);
                    }
                }
                icraft_ui::Op::Icon(_) => {}
            }
        }
    }

    pub(super) fn draw_dota_hud(&self, batch: &mut HudBatch) {
        if self.dota_pick.is_some() {
            let (w, h) = (batch.width, batch.height);
            let probe = self.dota_pick_paint(w, h, self.cursor, None);
            let hover = probe
                .as_ref()
                .and_then(|p| icraft_ui::click_at(p, self.cursor.0, self.cursor.1))
                .and_then(|a| a.strip_prefix("pick:"))
                .map(str::to_owned);
            let paint = if hover.is_some() {
                self.dota_pick_paint(w, h, self.cursor, hover)
            } else {
                probe
            };
            if let Some(paint) = paint {
                self.draw_paint(batch, &paint);
            }
            return;
        }
        let Some(s) = self.dota.as_ref() else { return };
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        let has_icon = |src: &str| atlas.sprites.contains_key(&format!("gui/{src}"));
        if let Some(panorama_paint) = self.dota_ui_paint(batch) {
            self.draw_paint(batch, &panorama_paint);
            let (html, css) = horizon_dota::hud::document_overlay(s, w, h, &has_icon);
            let paint = self.paint_rich(&html, &css, w, h, self.cursor, batch.scale);
            self.draw_paint(batch, &paint);
        } else {
            let (html, css) = horizon_dota::hud::document(s, w, h, &has_icon);
            let paint = self.paint_rich(&html, &css, w, h, self.cursor, batch.scale);
            self.draw_paint(batch, &paint);
        }
        if self.dota_shop.is_some() {
            let probe = self.dota_shop_paint(w, h, self.cursor, None);
            let hover = probe
                .as_ref()
                .and_then(|p| icraft_ui::click_at(p, self.cursor.0, self.cursor.1))
                .filter(|a| a.starts_with("buy:"))
                .map(str::to_owned);
            let shop = if hover.is_some() {
                self.dota_shop_paint(w, h, self.cursor, hover)
            } else {
                probe
            };
            if let Some(shop) = shop {
                self.draw_paint(batch, &shop);
            }
        }
    }
}

fn hud_color(argb: u32) -> u32 {
    (argb & 0xFF00_FF00) | ((argb >> 16) & 0xFF) | ((argb & 0xFF) << 16)
}
