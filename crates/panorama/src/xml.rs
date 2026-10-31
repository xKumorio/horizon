#[derive(Clone, Debug, Default)]
pub struct Xml {
    pub tag: String,
    pub attrs: Vec<(String, String)>,
    pub children: Vec<Xml>,
}

impl Xml {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

struct Cursor<'a> {
    text: &'a [u8],
    at: usize,
}

impl Cursor<'_> {
    fn starts(&self, s: &str) -> bool {
        self.text[self.at..].starts_with(s.as_bytes())
    }

    fn skip_space(&mut self) {
        while self.at < self.text.len() && self.text[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn skip_until(&mut self, end: &str) {
        while self.at < self.text.len() && !self.starts(end) {
            self.at += 1;
        }
        self.at = (self.at + end.len()).min(self.text.len());
    }

    fn name(&mut self) -> String {
        let start = self.at;
        while self.at < self.text.len() {
            let c = self.text[self.at];
            if c.is_ascii_whitespace() || c == b'=' || c == b'>' || c == b'/' {
                break;
            }
            self.at += 1;
        }
        String::from_utf8_lossy(&self.text[start..self.at]).into_owned()
    }
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn element(c: &mut Cursor) -> Option<Xml> {
    c.at += 1;
    let tag = c.name();
    let mut node = Xml { tag, ..Xml::default() };
    loop {
        c.skip_space();
        if c.at >= c.text.len() {
            return None;
        }
        if c.starts("/>") {
            c.at += 2;
            return Some(node);
        }
        if c.starts(">") {
            c.at += 1;
            break;
        }
        let key = c.name();
        c.skip_space();
        let mut value = String::new();
        if c.starts("=") {
            c.at += 1;
            c.skip_space();
            let quote = c.text.get(c.at).copied()?;
            c.at += 1;
            let start = c.at;
            while c.at < c.text.len() && c.text[c.at] != quote {
                c.at += 1;
            }
            value = unescape(&String::from_utf8_lossy(&c.text[start..c.at]));
            c.at += 1;
        }
        if key.is_empty() {
            c.at += 1;
            continue;
        }
        node.attrs.push((key, value));
    }
    loop {
        while c.at < c.text.len() && c.text[c.at] != b'<' {
            c.at += 1;
        }
        if c.at >= c.text.len() {
            return Some(node);
        }
        if c.starts("<!--") {
            c.skip_until("-->");
        } else if c.starts("<?") {
            c.skip_until("?>");
        } else if c.starts("</") {
            c.skip_until(">");
            return Some(node);
        } else if let Some(child) = element(c) {
            node.children.push(child);
        } else {
            return Some(node);
        }
    }
}

pub fn parse(text: &str) -> Option<Xml> {
    let mut c = Cursor { text: text.as_bytes(), at: 0 };
    loop {
        while c.at < c.text.len() && c.text[c.at] != b'<' {
            c.at += 1;
        }
        if c.at >= c.text.len() {
            return None;
        }
        if c.starts("<!--") {
            c.skip_until("-->");
        } else if c.starts("<?") || c.starts("<!") {
            c.skip_until(">");
        } else {
            return element(&mut c);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_reads_nested_panels_attributes_and_skips_comments() {
        let x = parse("<!-- c --><root><styles><include src=\"a\" /></styles><Panel id=\"x\" class=\"a b\"><Label text=\"hi &amp; you\"/></Panel></root>").unwrap();
        assert_eq!(x.tag, "root");
        assert_eq!(x.children[0].children[0].attr("src"), Some("a"));
        let panel = &x.children[1];
        assert_eq!(panel.attr("class"), Some("a b"));
        assert_eq!(panel.children[0].attr("text"), Some("hi & you"));
    }
}
