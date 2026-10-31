use std::collections::HashMap;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Compound {
    pub tag: Option<String>,
    pub id: Option<String>,
    pub classes: Vec<String>,
    pub not_classes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Selector {
    pub parts: Vec<(bool, Compound)>,
}

#[derive(Clone, Debug)]
pub struct Rule {
    pub sheet: u32,
    pub selector: Selector,
    pub specificity: u32,
    pub order: u32,
    pub decls: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct Sheet {
    pub rules: Vec<Rule>,
    pub vars: HashMap<String, String>,
}

fn strip_comments(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if b[i] == b'/' && b.get(i + 1) == Some(&b'/') && (i == 0 || b[i - 1] != b':') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn parse_compound(text: &str) -> Option<Compound> {
    let mut c = Compound::default();
    let mut rest = text;
    let mut first = true;
    while !rest.is_empty() {
        let (kind, tail) = match rest.as_bytes()[0] {
            b'#' => ('#', &rest[1..]),
            b'.' => ('.', &rest[1..]),
            b':' => {
                let Some(inner) = rest.strip_prefix(":not(") else { return None };
                let close = inner.find(')')?;
                let cls = inner[..close].strip_prefix('.')?;
                c.not_classes.push(cls.to_string());
                rest = &inner[close + 1..];
                continue;
            }
            _ => ('t', rest),
        };
        let end = tail.find(['#', '.', ':']).unwrap_or(tail.len());
        let name = &tail[..end];
        rest = &tail[end..];
        match kind {
            '#' => c.id = Some(name.to_string()),
            '.' => c.classes.push(name.to_string()),
            _ => {
                if !first || name.is_empty() {
                    return None;
                }
                if name != "*" {
                    c.tag = Some(name.to_string());
                }
            }
        }
        first = false;
    }
    Some(c)
}

fn parse_selector(text: &str) -> Option<Selector> {
    let spaced = text.replace('>', " > ");
    let mut parts = Vec::new();
    let mut direct = false;
    for tok in spaced.split_whitespace() {
        if tok == ">" {
            direct = true;
            continue;
        }
        parts.push((direct, parse_compound(tok)?));
        direct = false;
    }
    if parts.is_empty() {
        None
    } else {
        Some(Selector { parts })
    }
}

fn specificity(sel: &Selector) -> u32 {
    let mut s = 0;
    for (_, c) in &sel.parts {
        s += c.id.is_some() as u32 * 10000 + (c.classes.len() + c.not_classes.len()) as u32 * 100 + c.tag.is_some() as u32;
    }
    s
}

fn block_end(b: &[u8], mut i: usize) -> usize {
    let mut depth = 0;
    while i < b.len() {
        match b[i] {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    b.len()
}

fn push_decl(s: &str, out: &mut Vec<(String, String)>) {
    if let Some((k, v)) = s.split_once(':') {
        let k = k.trim();
        if !k.is_empty() {
            out.push((k.to_string(), v.trim().to_string()));
        }
    }
}

pub fn parse_decls(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut start = 0;
    for (i, &c) in body.as_bytes().iter().enumerate() {
        match c {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b';' if depth == 0 => {
                push_decl(&body[start..i], &mut out);
                start = i + 1;
            }
            _ => {}
        }
    }
    push_decl(&body[start..], &mut out);
    out
}

impl Sheet {
    pub fn parse(text: &str) -> Sheet {
        let mut sheet = Sheet::default();
        sheet.add(text, 0);
        sheet
    }

    pub fn add(&mut self, text: &str, sheet_id: u32) {
        let clean = strip_comments(text);
        let b = clean.as_bytes();
        let mut i = 0;
        while i < b.len() {
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= b.len() {
                break;
            }
            if b[i] == b'@' {
                let line_end = clean[i..].find(['\n', '{']).map_or(b.len(), |p| i + p);
                let braced = b.get(line_end) == Some(&b'{');
                if clean[i..].starts_with("@define") && !braced {
                    let end = clean[i..].find(';').map_or(b.len(), |p| i + p);
                    let line = clean[i + 7..end].trim();
                    if let Some((k, v)) = line.split_once(':') {
                        self.vars.insert(k.trim().to_string(), v.trim().to_string());
                    }
                    i = end + 1;
                } else if braced {
                    i = block_end(b, line_end);
                } else {
                    i = line_end + 1;
                }
                continue;
            }
            let Some(open) = clean[i..].find('{').map(|p| i + p) else { break };
            let end = block_end(b, open);
            let head = &clean[i..open];
            let body = &clean[open + 1..end.saturating_sub(1).max(open + 1)];
            i = end;
            let decls = parse_decls(body);
            for part in head.split(',') {
                let Some(selector) = parse_selector(part.trim()) else { continue };
                let order = self.rules.len() as u32;
                self.rules.push(Rule { sheet: sheet_id, specificity: specificity(&selector), selector, order, decls: decls.clone() });
            }
        }
    }

    pub fn resolve(&self, value: &str) -> String {
        if self.vars.is_empty() || !value.contains(|c: char| c.is_ascii_alphabetic()) {
            return value.to_string();
        }
        let mut out = String::new();
        let mut word = String::new();
        let flush = |word: &mut String, out: &mut String| {
            if !word.is_empty() {
                match self.vars.get(word.as_str()) {
                    Some(v) => out.push_str(v),
                    None => out.push_str(word),
                }
                word.clear();
            }
        };
        for c in value.chars() {
            if c.is_ascii_alphanumeric() || c == '_' {
                word.push(c);
            } else {
                flush(&mut word, &mut out);
                out.push(c);
            }
        }
        flush(&mut word, &mut out);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_rules_variables_and_skips_pseudo_states_and_keyframes() {
        let s = Sheet::parse("@define gold: #ffcc33;\n#a .b > Label.c { color: gold; width: 5px } .x:hover { color: red } @keyframes 'k' { from { a: b } } #z { height: 2px }");
        assert_eq!(s.rules.len(), 2);
        assert_eq!(s.rules[0].selector.parts.len(), 3);
        assert!(s.rules[0].selector.parts[2].0);
        assert_eq!(s.rules[0].selector.parts[2].1.tag.as_deref(), Some("Label"));
        assert_eq!(s.resolve("gold"), "#ffcc33");
        assert_eq!(s.rules[1].decls[0], ("height".into(), "2px".into()));
    }

    #[test]
    fn it_keeps_semicolons_inside_functions_together() {
        let d = parse_decls("background-image: url(\"a;b\"); width: 1px");
        assert_eq!(d.len(), 2);
    }
}
