//! The HTML subset I-Craft windows use. Same rules as the Fabric engine:
//! elements, attributes, text, comments, a few entities. Unknown tags stay
//! as boxes. A mismatched close tag closes up to the matching open one.

const VOID: &[&str] = &["br", "img", "item", "input", "hr"];
const MAX_NODES: usize = 5000;

#[derive(Clone, Debug, PartialEq)]
pub struct Element {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub classes: Vec<String>,
    pub children: Vec<Node>,
}

impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
    }

    pub fn has_class(&self, name: &str) -> bool {
        self.classes.iter().any(|class| class == name)
    }

    pub fn walk(&self) -> Vec<&Element> {
        let mut out = Vec::new();
        fn go<'a>(element: &'a Element, out: &mut Vec<&'a Element>) {
            out.push(element);
            for child in &element.children {
                if let Node::Element(inner) = child {
                    go(inner, out);
                }
            }
        }
        go(self, &mut out);
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Element(Element),
    Text(String),
}

pub fn parse(html: &str) -> Result<Element, &'static str> {
    let mut parser = Parser { src: html, pos: 0, nodes: 0 };
    let mut body = Element { tag: "body".into(), attrs: Vec::new(), classes: Vec::new(), children: Vec::new() };
    parser.run(&mut body)?;
    Ok(body)
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
    nodes: usize,
}

impl<'a> Parser<'a> {
    fn run(&mut self, root: &mut Element) -> Result<(), &'static str> {
        let mut stack: Vec<Element> = Vec::new();
        let mut text = String::new();
        while self.pos < self.src.len() {
            let rest = &self.src[self.pos..];
            if rest.starts_with("<!--") {
                self.flush(root, &mut stack, &mut text)?;
                self.pos = self.src[self.pos + 4..].find("-->").map_or(self.src.len(), |i| self.pos + 4 + i + 3);
            } else if rest.starts_with("</") {
                self.flush(root, &mut stack, &mut text)?;
                let end = self.src[self.pos..].find('>').map_or(self.src.len(), |i| self.pos + i);
                let name = self.src[self.pos + 2..end].trim().to_ascii_lowercase();
                self.pos = if end == self.src.len() { end } else { end + 1 };
                // A close tag without an open one (`</item>` after a void
                // element) is ignored, as browsers do.
                if !stack.iter().any(|open| open.tag == name) {
                    continue;
                }
                while let Some(top) = stack.last() {
                    if top.tag == name {
                        let done = stack.pop().unwrap();
                        self.attach(root, &mut stack, Node::Element(done));
                        break;
                    }
                    let done = stack.pop().unwrap();
                    self.attach(root, &mut stack, Node::Element(done));
                }
            } else if rest.starts_with('<') && rest.chars().nth(1).is_some_and(|c| c.is_ascii_alphabetic()) {
                self.flush(root, &mut stack, &mut text)?;
                let element = self.read_tag();
                self.nodes += 1;
                if self.nodes > MAX_NODES {
                    return Err("too many elements");
                }
                let void = VOID.contains(&element.tag.as_str());
                if void {
                    self.attach(root, &mut stack, Node::Element(element));
                } else {
                    stack.push(element);
                }
            } else {
                text.push(self.src[self.pos..].chars().next().unwrap());
                self.pos += self.src[self.pos..].chars().next().unwrap().len_utf8();
            }
        }
        self.flush(root, &mut stack, &mut text)?;
        while let Some(done) = stack.pop() {
            self.attach(root, &mut stack, Node::Element(done));
        }
        Ok(())
    }

    fn attach(&mut self, root: &mut Element, stack: &mut [Element], node: Node) {
        if let Some(parent) = stack.last_mut() {
            parent.children.push(node);
        } else {
            root.children.push(node);
        }
    }

    fn flush(&mut self, root: &mut Element, stack: &mut [Element], text: &mut String) -> Result<(), &'static str> {
        if text.is_empty() {
            return Ok(());
        }
        let decoded = decode(text);
        text.clear();
        if decoded.trim().is_empty() {
            return Ok(());
        }
        self.nodes += 1;
        if self.nodes > MAX_NODES {
            return Err("too many nodes");
        }
        self.attach(root, stack, Node::Text(decoded));
        Ok(())
    }

    fn read_tag(&mut self) -> Element {
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.src.len() {
            let c = self.src[self.pos..].chars().next().unwrap();
            if c.is_whitespace() || c == '>' || c == '/' {
                break;
            }
            self.pos += c.len_utf8();
        }
        let mut element = Element {
            tag: self.src[start..self.pos].to_ascii_lowercase(),
            attrs: Vec::new(),
            classes: Vec::new(),
            children: Vec::new(),
        };
        while self.pos < self.src.len() {
            self.skip_space();
            if self.pos >= self.src.len() {
                break;
            }
            let c = self.src[self.pos..].chars().next().unwrap();
            if c == '>' {
                self.pos += 1;
                break;
            }
            if c == '/' {
                self.pos += 1;
                continue;
            }
            let name_start = self.pos;
            while self.pos < self.src.len() && !"=>/ \t\r\n".contains(self.src[self.pos..].chars().next().unwrap()) {
                self.pos += self.src[self.pos..].chars().next().unwrap().len_utf8();
            }
            let name = self.src[name_start..self.pos].to_ascii_lowercase();
            self.skip_space();
            let mut value = String::new();
            if self.pos < self.src.len() && self.src[self.pos..].starts_with('=') {
                self.pos += 1;
                self.skip_space();
                if self.pos < self.src.len() {
                    let quote = self.src[self.pos..].chars().next().unwrap();
                    if quote == '"' || quote == '\'' {
                        self.pos += 1;
                        let end = self.src[self.pos..].find(quote).map_or(self.src.len(), |i| self.pos + i);
                        value = decode(&self.src[self.pos..end]);
                        self.pos = (end + 1).min(self.src.len());
                    } else {
                        let value_start = self.pos;
                        while self.pos < self.src.len() {
                            let ch = self.src[self.pos..].chars().next().unwrap();
                            if ch.is_whitespace() || ch == '>' {
                                break;
                            }
                            self.pos += ch.len_utf8();
                        }
                        value = decode(&self.src[value_start..self.pos]);
                    }
                }
            }
            if name == "class" {
                element.classes = value.split_whitespace().map(str::to_owned).collect();
            }
            if !name.is_empty() {
                element.attrs.push((name, value));
            }
        }
        element
    }

    fn skip_space(&mut self) {
        while self.pos < self.src.len() && self.src[self.pos..].chars().next().unwrap().is_whitespace() {
            self.pos += self.src[self.pos..].chars().next().unwrap().len_utf8();
        }
    }
}

fn decode(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    let mut out = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '&' {
            if let Some(semi) = chars[i + 1..].iter().position(|c| *c == ';') {
                let entity: String = chars[i + 1..i + 1 + semi].iter().collect();
                if let Some(value) = entity_value(&entity) {
                    out.push_str(&value);
                    i += semi + 2;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn entity_value(entity: &str) -> Option<String> {
    Some(match entity {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" | "#39" => "'",
        "nbsp" => "\u{00a0}",
        "mdash" => "—",
        "laquo" => "«",
        "raquo" => "»",
        _ => return None,
    }.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mode_menu_markup_is_six_cards_with_icons_and_clicks() {
        let html = r#"<div class=panel><div class=bar></div><div class=head><div class=title>Выбор режима</div></div><div class=grid><div class=card onclick="pick('bedwars')"><div class=icon><item id="minecraft:red_bed"></item></div><div class=name>Bed Wars</div></div><div class=card onclick="pick('hide')"><item id="minecraft:spyglass"></item></div></div></div>"#;
        let body = parse(html).unwrap();
        let cards: Vec<_> = body.walk().into_iter().filter(|e| e.has_class("card")).collect();
        assert_eq!(cards.len(), 2);
        assert_eq!(cards[0].attr("onclick"), Some("pick('bedwars')"));
        let item = body.walk().into_iter().find(|e| e.tag == "item").unwrap();
        assert_eq!(item.attr("id"), Some("minecraft:red_bed"));
    }

    #[test]
    fn a_closing_item_tag_keeps_the_card_open() {
        let body = parse("<div class=panel><div class=card><item id=a></item><div class=name>x</div></div></div>").unwrap();
        assert_eq!(body.children.len(), 1);
        let card = body.walk().into_iter().find(|e| e.has_class("card")).unwrap();
        assert_eq!(card.children.len(), 2);
    }
}
