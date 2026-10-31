//! Screens the server serves. The mode menu is the same document Gate sends.

use crate::{ClientMessage, ServerMessage};
use serde_json::{json, Value};

const HTML: &str = concat!(
    "<div class=panel><div class=bar></div>",
    "<div class=head><div class=title>Выбор режима</div>",
    "<div class=sub>Выберите режим, чтобы перейти на его сервер</div></div>",
    "<div class=grid>",
    "<div class=card onclick=\"pick('bedwars')\"><div class=icon><item id=\"minecraft:red_bed\"></item></div><div class=info><div class=name>Bed Wars</div><div class=desc>Защитите свою кровать и сломайте чужие</div><div class=soon>СКОРО</div></div></div>",
    "<div class=card onclick=\"pick('skywars')\"><div class=icon><item id=\"minecraft:ender_eye\"></item></div><div class=info><div class=name>Sky Wars</div><div class=desc>Острова в небе: соберите лут и останьтесь последним</div><div class=soon>СКОРО</div></div></div>",
    "<div class=card onclick=\"pick('duels')\"><div class=icon><item id=\"minecraft:iron_sword\"></item></div><div class=info><div class=name>Дуэли</div><div class=desc>Один на один, равные наборы</div><div class=soon>СКОРО</div></div></div>",
    "<div class=card onclick=\"pick('parkour')\"><div class=icon><item id=\"minecraft:feather\"></item></div><div class=info><div class=name>Паркур</div><div class=desc>Трассы на время с таблицей рекордов</div><div class=soon>СКОРО</div></div></div>",
    "<div class=card onclick=\"pick('build')\"><div class=icon><item id=\"minecraft:bricks\"></item></div><div class=info><div class=name>Битва строителей</div><div class=desc>Постройте на тему, остальные оценят</div><div class=soon>СКОРО</div></div></div>",
    "<div class=card onclick=\"pick('hide')\"><div class=icon><item id=\"minecraft:spyglass\"></item></div><div class=info><div class=name>Прятки</div><div class=desc>Притворитесь блоком и не попадитесь</div><div class=soon>СКОРО</div></div></div>",
    "</div><div class=foot><span id=notice>M или Esc — закрыть</span></div></div>"
);

const CSS: &str = "
.panel { display: flex; flex-direction: column; width: 560px; max-width: 96%; max-height: 94%; background: rgba(18, 19, 24, 0.94); border: 1px solid #2b2e38; }
.bar { height: 2px; background: #ebce96; }
.head { padding: 10px 12px 6px 12px; }
.title { color: #ebce96; font-weight: bold; }
.sub { color: #8c909c; margin-top: 3px; }
.grid { display: flex; flex-wrap: wrap; gap: 8px; padding: 4px 12px 8px 12px; }
.card { display: flex; align-items: center; gap: 8px; width: 172px; flex-grow: 1; padding: 8px; background: #1a1c23; border: 1px solid #2b2e38; }
.card:hover { background: #22252e; border-color: #ebce96; }
.icon { display: flex; align-items: center; justify-content: center; width: 28px; height: 28px; background: #101116; }
.name { color: #f2f2f5; font-weight: bold; }
.desc { color: #8c909c; margin-top: 3px; }
.soon { color: #ebce96; }
.live { display: flex; align-items: center; gap: 4px; color: #7fd18b; margin-top: 3px; }
.dot { width: 5px; height: 5px; background: #7fd18b; }
.party { color: #8c909c; margin-top: 2px; }
.card.open { border-color: #4a5a3a; }
.foot { padding: 2px 12px 8px 12px; color: #5e6270; }
.foot.msg { color: #ebce96; }
";

const NAMES: &[(&str, &str)] = &[
    ("bedwars", "Bed Wars"),
    ("skywars", "Sky Wars"),
    ("duels", "Дуэли"),
    ("parkour", "Паркур"),
    ("build", "Битва строителей"),
    ("hide", "Прятки"),
];

/// What the server sends back for one client message, if it knows the screen.
pub fn reply(json: &str) -> Option<String> {
    match crate::client_message(json)? {
        ClientMessage::Hello => Some(serde_json::to_string(&binds()).ok()?),
        ClientMessage::OpenRequest { screen } if screen == "minigames" => {
            Some(serde_json::to_string(&window()).ok()?)
        }
        ClientMessage::Event { window, name, data } if window == "minigames" && name == "select" => {
            let id = data.as_ref().and_then(|v| v.get("id")).and_then(Value::as_str).unwrap_or("");
            let name = NAMES.iter().find(|(key, _)| *key == id).map(|(_, name)| *name)?;
            let message = ServerMessage::Data {
                id: "minigames".into(),
                data: json!({ "notice": format!("«{name}» скоро откроется — следите за новостями") }),
            };
            serde_json::to_string(&message).ok()
        }
        _ => None,
    }
}

pub fn binds() -> ServerMessage {
    ServerMessage::Binds {
        binds: vec![crate::ServerBind {
            id: "minigames".into(),
            label: "Выбор режима".into(),
            key: "key.keyboard.m".into(),
            screen: "minigames".into(),
            item: Some("minecraft:compass".into()),
        }],
    }
}

pub fn window() -> ServerMessage {
    ServerMessage::Window {
        id: "minigames".into(),
        title: "Выбор режима".into(),
        html: HTML.into(),
        css: CSS.into(),
        script: "function pick(id) { ui.send('select', { id: id }); }".into(),
        data: None,
        close_on_key: true,
    }
}

/// The menu with Sky Wars open: no "soon", a live line (queue and matches)
/// and that a party plays together.
pub fn window_live(skywars: &str) -> ServerMessage {
    let soon = "<div class=desc>Острова в небе: соберите лут и останьтесь последним</div><div class=soon>СКОРО</div>";
    let live = format!(
        "<div class=desc>Острова в небе: соберите лут и останьтесь последним</div><div class=live><div class=dot></div>{}</div><div class=party>Дуэты · пати попадает в одну команду</div>",
        escape(skywars)
    );
    let html = HTML.replacen("<div class=card onclick=\"pick('skywars')\">", "<div class=\"card open\" onclick=\"pick('skywars')\">", 1).replacen(soon, &live, 1);
    let ServerMessage::Window { id, title, css, script, data, close_on_key, .. } = window() else { unreachable!() };
    ServerMessage::Window { id, title, html, css, script, data, close_on_key }
}

/// The Sky Wars variants: (id, name, about, live line) each; a pick sends
/// `select` with the id, like the mode menu.
pub fn skywars_modes(modes: &[(String, String, String, String)]) -> ServerMessage {
    let icons = ["minecraft:iron_sword", "minecraft:golden_sword", "minecraft:diamond_sword", "minecraft:netherite_sword"];
    let mut html = String::from("<div class=panel><div class=bar></div><div class=head><div class=title>Sky Wars</div><div class=sub>Выберите формат — пати попадает в одну команду</div></div><div class=grid>");
    for (k, (id, name, about, live)) in modes.iter().enumerate() {
        html += &format!(
            "<div class=\"card open\" onclick=\"pick('{}')\"><div class=icon><item id=\"{}\"></item></div><div class=info><div class=name>{}</div><div class=desc>{}</div><div class=live><div class=dot></div>{}</div></div></div>",
            escape(id),
            icons[k % icons.len()],
            escape(name),
            escape(about),
            escape(live)
        );
    }
    html += "</div><div class=foot><span id=notice>Esc — назад</span></div></div>";
    let ServerMessage::Window { css, script, close_on_key, .. } = window() else { unreachable!() };
    ServerMessage::Window { id: "minigames".into(), title: "Sky Wars".into(), html, css, script, data: None, close_on_key }
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn document() -> (&'static str, &'static str) {
    (HTML, CSS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asking_for_the_menu_returns_the_html_document() {
        let raw = reply(r#"{"type":"open_request","screen":"minigames"}"#).unwrap();
        let message = crate::server_message(&raw).unwrap();
        let crate::ServerMessage::Window { html, css, .. } = message else { panic!("window") };
        assert!(html.contains("pick('bedwars')"));
        assert!(css.contains(".card:hover"));
    }

    #[test]
    fn the_live_menu_opens_sky_wars() {
        let crate::ServerMessage::Window { html, .. } = window_live("В очереди: 2 · Играют: 4") else { panic!("window") };
        let card = html.split("pick('skywars')").nth(1).unwrap().split("</div></div></div>").next().unwrap();
        assert!(card.contains("В очереди: 2"));
        assert!(!card.contains("СКОРО"));
        assert!(html.contains("card open"));
        // The other modes are still to come.
        assert!(html.split("pick('bedwars')").nth(1).unwrap().contains("СКОРО"));
    }
}
