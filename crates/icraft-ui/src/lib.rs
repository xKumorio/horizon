//! The I-Craft window protocol (`icraft:ui`), the same messages Gate's
//! `UiWindows` sends and the Fabric client renders.
//!
//! One UTF-8 JSON object per message. The server owns the markup. The client
//! parses it, lays it out and draws it. Events go back as `event`.
//!
//! ```json
//! {"type":"window","id":"minigames","title":"Выбор режима","html":"…","css":"…","script":"…","closeOnKey":true}
//! {"type":"open_request","screen":"minigames"}
//! {"type":"event","window":"minigames","name":"select","data":{"id":"bedwars"}}
//! ```

mod html;
mod layout;
mod screens;

pub use html::{Element, Node, parse};
pub use layout::{click_at, inside, paint, paint_rich, paint_with, Click, Fill, Font, Gradient, Icon, Image, Label, Op, Paint, RichMeasure, ITEM, LINE};
pub use screens::{document, reply, skywars_modes, window, window_live};

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CHANNEL: &str = "icraft:ui";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Window {
        id: String,
        #[serde(default)]
        title: String,
        html: String,
        #[serde(default)]
        css: String,
        #[serde(default)]
        script: String,
        #[serde(default)]
        data: Option<Value>,
        #[serde(default, rename = "closeOnKey")]
        close_on_key: bool,
    },
    Data {
        id: String,
        data: Value,
    },
    Patch {
        id: String,
        selector: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        html: Option<String>,
        #[serde(default, rename = "addClass")]
        add_class: Option<String>,
        #[serde(default, rename = "removeClass")]
        remove_class: Option<String>,
    },
    Close {
        id: String,
    },
    Binds {
        binds: Vec<ServerBind>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerBind {
    pub id: String,
    pub label: String,
    pub key: String,
    pub screen: String,
    #[serde(default)]
    pub item: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello,
    OpenRequest {
        screen: String,
    },
    Event {
        window: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
    },
}

pub fn server_message(json: &str) -> Option<ServerMessage> {
    serde_json::from_str(json).ok()
}

pub fn client_message(json: &str) -> Option<ClientMessage> {
    serde_json::from_str(json).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_message_keeps_the_markup_for_the_client() {
        let raw = r#"{"type":"window","id":"minigames","title":"Выбор режима","html":"<div class=panel></div>","css":".panel{}","script":"function pick(id){ui.send('select',{id:id})}","closeOnKey":true}"#;
        let ServerMessage::Window { id, html, close_on_key, .. } = server_message(raw).unwrap() else {
            panic!("window");
        };
        assert_eq!(id, "minigames");
        assert!(html.contains("panel"));
        assert!(close_on_key);
    }

    #[test]
    fn the_client_asks_the_server_to_open_a_screen() {
        let message = ClientMessage::OpenRequest { screen: "minigames".into() };
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(json, r#"{"type":"open_request","screen":"minigames"}"#);
    }
}
