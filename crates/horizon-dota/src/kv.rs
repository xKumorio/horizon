use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Block(Kv),
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Kv {
    pub entries: Vec<(String, Value)>,
}

impl Kv {
    pub fn parse(text: &str) -> Result<Kv, String> {
        let mut lexer = Lexer {
            chars: text.char_indices().peekable(),
            text,
        };
        let mut kv = Kv::default();
        loop {
            match lexer.token()? {
                Token::End => break,
                Token::Directive => {
                    lexer.token()?;
                }
                Token::Text(key) => {
                    let value = lexer.value()?;
                    kv.entries.push((key, value));
                }
                Token::Close => return Err("unexpected }".into()),
                Token::Open => return Err("unexpected {".into()),
            }
        }
        Ok(kv)
    }

    pub fn parse_file(path: &Path) -> Result<Kv, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Kv::parse(&String::from_utf8_lossy(&bytes))
    }

    pub fn bases(text: &str) -> Vec<String> {
        text.lines()
            .filter_map(|line| line.trim().strip_prefix("#base"))
            .map(|rest| rest.trim().trim_matches('"').to_string())
            .collect()
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries
            .iter()
            .rev()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn str(&self, key: &str) -> Option<&str> {
        match self.get(key)? {
            Value::Str(s) => Some(s),
            Value::Block(_) => None,
        }
    }

    pub fn block(&self, key: &str) -> Option<&Kv> {
        match self.get(key)? {
            Value::Block(b) => Some(b),
            Value::Str(_) => None,
        }
    }

    pub fn f32(&self, key: &str) -> Option<f32> {
        first_number(self.str(key)?)
    }

    pub fn f32_or(&self, key: &str, default: f32) -> f32 {
        self.f32(key).unwrap_or(default)
    }

    pub fn int(&self, key: &str) -> Option<i32> {
        self.f32(key).map(|v| v as i32)
    }

    pub fn blocks(&self) -> impl Iterator<Item = (&str, &Kv)> {
        self.entries.iter().filter_map(|(k, v)| match v {
            Value::Block(b) => Some((k.as_str(), b)),
            Value::Str(_) => None,
        })
    }

    pub fn merge(&mut self, over: &Kv) {
        for (key, value) in &over.entries {
            match (self.entries.iter_mut().find(|(k, _)| k == key), value) {
                (Some((_, Value::Block(mine))), Value::Block(theirs)) => mine.merge(theirs),
                (Some(slot), _) => slot.1 = value.clone(),
                (None, _) => self.entries.push((key.clone(), value.clone())),
            }
        }
    }
}

pub fn first_number(text: &str) -> Option<f32> {
    text.split_whitespace().next()?.parse().ok()
}

pub fn numbers(text: &str) -> Vec<f32> {
    text.split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect()
}

enum Token {
    Text(String),
    Directive,
    Open,
    Close,
    End,
}

struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    text: &'a str,
}

impl Lexer<'_> {
    fn skip_space(&mut self) {
        loop {
            match self.chars.peek() {
                Some(&(_, c)) if c.is_whitespace() || c == '\u{feff}' => {
                    self.chars.next();
                }
                Some(&(i, '/')) if self.text[i..].starts_with("//") => {
                    while let Some(&(_, c)) = self.chars.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.chars.next();
                    }
                }
                _ => break,
            }
        }
    }

    fn token(&mut self) -> Result<Token, String> {
        self.skip_space();
        let Some(&(start, c)) = self.chars.peek() else {
            return Ok(Token::End);
        };
        match c {
            '{' => {
                self.chars.next();
                Ok(Token::Open)
            }
            '}' => {
                self.chars.next();
                Ok(Token::Close)
            }
            '"' => {
                self.chars.next();
                let mut out = String::new();
                loop {
                    match self.chars.next() {
                        None => return Err("unterminated string".into()),
                        Some((_, '"')) => break,
                        Some((_, '\\')) => match self.chars.next() {
                            Some((_, 'n')) => out.push('\n'),
                            Some((_, 't')) => out.push('\t'),
                            Some((_, other)) => out.push(other),
                            None => return Err("unterminated string".into()),
                        },
                        Some((_, c)) => out.push(c),
                    }
                }
                Ok(Token::Text(out))
            }
            '#' => {
                while let Some(&(_, c)) = self.chars.peek() {
                    if c.is_whitespace() {
                        break;
                    }
                    self.chars.next();
                }
                Ok(Token::Directive)
            }
            _ => {
                let mut end = start;
                while let Some(&(i, c)) = self.chars.peek() {
                    if c.is_whitespace() || c == '{' || c == '}' || c == '"' {
                        break;
                    }
                    end = i + c.len_utf8();
                    self.chars.next();
                }
                Ok(Token::Text(self.text[start..end].to_string()))
            }
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        match self.token()? {
            Token::Text(s) => Ok(Value::Str(s)),
            Token::Open => {
                let mut kv = Kv::default();
                loop {
                    match self.token()? {
                        Token::Close => return Ok(Value::Block(kv)),
                        Token::End => return Err("unclosed block".into()),
                        Token::Text(key) => {
                            let value = self.value()?;
                            kv.entries.push((key, value));
                        }
                        Token::Directive => {
                            self.token()?;
                        }
                        Token::Open => return Err("unexpected {".into()),
                    }
                }
            }
            Token::Close => Err("unexpected }".into()),
            Token::Directive => Err("unexpected directive".into()),
            Token::End => Err("missing value".into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_blocks_and_comments() {
        let kv = Kv::parse(
            "\"Root\" // c\n{\n\t\"a\" \"1 2 3\"\n\t\"B\"\n\t{\n\t\t\"c\" \"4\"\n\t}\n}\n",
        )
        .unwrap();
        let root = kv.block("root").unwrap();
        assert_eq!(root.str("a"), Some("1 2 3"));
        assert_eq!(root.block("b").unwrap().f32("c"), Some(4.0));
        assert_eq!(numbers(root.str("a").unwrap()), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn bare_tokens_and_directives() {
        let kv = Kv::parse("#base \"x.txt\"\nkey value\n").unwrap();
        assert_eq!(kv.str("key"), Some("value"));
        assert_eq!(Kv::bases("#base \"x.txt\"\n"), vec!["x.txt".to_string()]);
    }

    #[test]
    fn merge_overrides_and_extends() {
        let mut a = Kv::parse("r { x 1 y { p 1 } }").unwrap();
        let b = Kv::parse("r { x 2 y { q 3 } z 9 }").unwrap();
        a.merge(&b);
        let r = a.block("r").unwrap();
        assert_eq!(r.f32("x"), Some(2.0));
        assert_eq!(r.block("y").unwrap().f32("p"), Some(1.0));
        assert_eq!(r.block("y").unwrap().f32("q"), Some(3.0));
        assert_eq!(r.f32("z"), Some(9.0));
    }

    #[test]
    fn unterminated_is_an_error() {
        assert!(Kv::parse("r { x 1").is_err());
        assert!(Kv::parse("r \"abc").is_err());
    }
}
