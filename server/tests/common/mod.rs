#![allow(dead_code)] // each test file uses part of it

use parinfer_lsp::Server;
use serde_json::{json, Value};

pub type At = (usize, usize); // line, column in UTF-16 units, as LSP has it

/// Stands in for Zed: sends changes, applies the server's fixes, echoes them
/// back as changes, and answers. It keeps a cursor the way Zed does, so text
/// inserted right at the cursor pushes it along.
pub struct Zed {
    server: Server,
    pub text: String,
    cursor: usize, // byte offset
    pub autoclose: bool,
    version: i64,
    pub fixes: usize,
    pub messages: Vec<String>,
    undo: Vec<String>,
    // Fixes held back, to type while they're on their way.
    pub hold: bool,
    held: Vec<Value>,
}

impl Zed {
    pub fn open(language: &str, text: &str) -> Zed {
        let mut zed = Zed {
            server: Server::default(),
            text: text.to_string(),
            cursor: 0,
            autoclose: true,
            version: 0,
            fixes: 0,
            messages: vec![],
            undo: vec![],
            hold: false,
            held: vec![],
        };
        let capabilities = json!({"workspace": {"workspaceEdit": {"snippetEditSupport": true}}});
        zed.send(json!({"id": 0, "method": "initialize",
                        "params": {"capabilities": capabilities}}));
        zed.send(
            json!({"method": "textDocument/didOpen", "params": {"textDocument": {
                "uri": "file:///test", "languageId": language, "version": 0, "text": text,
            }}}),
        );
        zed
    }

    pub fn type_at(&mut self, at: At, text: &str) {
        self.replace(at, at, text);
    }

    pub fn replace(&mut self, from: At, to: At, text: &str) {
        self.change(&[(from, to, text)]);
    }

    /// Several changes in one notification, each against the text before it.
    /// The cursor ends up after the last one.
    pub fn change(&mut self, changes: &[(At, At, &str)]) {
        let mut sent = vec![];
        for &(from, to, text) in changes {
            let (start, end) = (offset(&self.text, from), offset(&self.text, to));
            self.text.replace_range(start..end, text);
            self.cursor = start + text.len();
            sent.push(json!({"range": range(from, to), "text": text}));
        }
        self.notify(sent);
    }

    pub fn move_to(&mut self, at: At) {
        self.cursor = offset(&self.text, at);
    }

    pub fn cursor(&self) -> At {
        position(&self.text, self.cursor)
    }

    /// Types at the cursor, a key at a time. With auto-closing on, an opening
    /// bracket brings its closer, and typing a closer steps over one.
    pub fn type_keys(&mut self, keys: &str) {
        for key in keys.chars() {
            let next = self.text[self.cursor..].chars().next();
            if self.autoclose && ")]}\"".contains(key) && next == Some(key) {
                self.cursor += 1;
                continue;
            }
            let inserted = match key {
                '(' if self.autoclose => "()".to_string(),
                '[' if self.autoclose => "[]".to_string(),
                '{' if self.autoclose => "{}".to_string(),
                '"' if self.autoclose => "\"\"".to_string(),
                _ => key.to_string(),
            };
            let at = self.cursor();
            self.text.insert_str(self.cursor, &inserted);
            self.cursor += key.len_utf8();
            self.notify(vec![json!({"range": range(at, at), "text": inserted})]);
        }
    }

    /// Undoes the last fix, the way Zed's undo would.
    pub fn undo(&mut self) {
        self.text = self.undo.pop().expect("a fix to undo");
        let text = self.text.clone();
        self.notify(vec![json!({"text": text})]);
    }

    /// Lets the held fixes through, then answers them.
    pub fn release(&mut self, applied: bool) {
        for fix in std::mem::take(&mut self.held) {
            if applied {
                self.apply(&fix);
            }
            self.send(json!({"id": fix["id"], "result": {"applied": applied}}));
        }
    }

    pub fn send(&mut self, msg: Value) -> Vec<Value> {
        let replies = self.server.handle(&msg);
        for reply in &replies {
            match reply["method"].as_str() {
                Some("workspace/applyEdit") if self.hold => self.held.push(reply.clone()),
                Some("workspace/applyEdit") => {
                    self.apply(reply);
                    self.send(json!({"id": reply["id"], "result": {"applied": true}}));
                }
                Some("window/showMessage") => {
                    let message = reply["params"]["message"].as_str().unwrap();
                    self.messages.push(message.to_string());
                }
                _ => {}
            }
        }
        replies
    }

    fn notify(&mut self, changes: Vec<Value>) {
        self.version += 1;
        self.send(json!({"method": "textDocument/didChange", "params": {
            "textDocument": {"uri": "file:///test", "version": self.version},
            "contentChanges": changes,
        }}));
    }

    // Edits come in order, positioned in the text they were made for: apply
    // them last first so the earlier positions still hold.
    fn apply(&mut self, fix: &Value) {
        let doc = &fix["params"]["edit"]["documentChanges"][0];
        self.fixes += 1;
        self.undo.push(self.text.clone());
        let mut echo = vec![];
        for edit in doc["edits"].as_array().unwrap().iter().rev() {
            let (from, to) = (at(&edit["range"]["start"]), at(&edit["range"]["end"]));
            let (start, end) = (offset(&self.text, from), offset(&self.text, to));
            let (text, cursor) = match edit["snippet"]["value"].as_str() {
                Some(snippet) => unsnippet(snippet),
                None => (edit["newText"].as_str().unwrap().to_string(), None),
            };
            self.text.replace_range(start..end, &text);
            self.cursor = match cursor {
                Some(cursor) => start + cursor,
                // Zed's cursor sticks to the text after it.
                None if start == end && self.cursor == start => start + text.len(),
                None if self.cursor >= end && self.cursor > start => {
                    self.cursor - (end - start) + text.len()
                }
                None if self.cursor > start => start + text.len(),
                None => self.cursor,
            };
            echo.push(json!({"range": range(from, to), "text": text}));
        }
        self.notify(echo);
    }
}

// A snippet's text, and where its `$0` is.
fn unsnippet(snippet: &str) -> (String, Option<usize>) {
    let (mut text, mut cursor) = (String::new(), None);
    let mut chars = snippet.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => text.extend(chars.next()),
            '$' if chars.peek() == Some(&'0') => {
                chars.next();
                cursor = Some(text.len());
            }
            _ => text.push(c),
        }
    }
    (text, cursor)
}

fn at(pos: &Value) -> At {
    (
        pos["line"].as_u64().unwrap() as usize,
        pos["character"].as_u64().unwrap() as usize,
    )
}

fn range(from: At, to: At) -> Value {
    json!({"start": {"line": from.0, "character": from.1},
           "end": {"line": to.0, "character": to.1}})
}

fn offset(text: &str, (line, column): At) -> usize {
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    let mut units = 0;
    for (i, c) in text[start..].char_indices() {
        if units == column {
            return start + i;
        }
        units += c.len_utf16();
    }
    text.len()
}

fn position(text: &str, offset: usize) -> At {
    let before = &text[..offset];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    (
        before.matches('\n').count(),
        before[line_start..].encode_utf16().count(),
    )
}
