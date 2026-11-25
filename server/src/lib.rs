//! Parinfer as a language server. Zed extensions can't touch edits as they
//! happen, but a language server hears about every change and can send edits
//! back. So: run parinfer on each change and send back what it fixed.

use parinfer_rust::{parinfer, types::Request};
use serde_json::{json, Value};
use similar::{capture_diff_slices, Algorithm, DiffTag};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

type Pos = (usize, usize); // line, and column the way parinfer counts them

#[derive(Default)]
struct Doc {
    text: String,
    version: i64,
    language: String,
    // LSP never says where the cursor is, so we guess from the last change.
    cursor: Pos,
    prev_cursor: Pos,
    // Our fix is out: the text it makes, and whether Zed has answered.
    fix: Option<String>,
    waiting: bool,
    typed_while_waiting: bool,
    // The text before our last fix. Seeing it again means the fix was undone.
    unfixed: Option<String>,
    // Off while indentation and parens disagree somewhere: parinfer trusts the
    // indentation, so it would move parens there and change the code.
    off: bool,
}

#[derive(Default)]
pub struct Server {
    docs: HashMap<String, Doc>,
    requests: HashMap<i64, String>,
    next_id: i64,
    out: Vec<Value>,
    snippets: bool, // whether the editor takes snippet edits, which place the cursor
}

impl Server {
    /// Takes one message from Zed and returns the messages to send back.
    pub fn handle(&mut self, msg: &Value) -> Vec<Value> {
        let params = &msg["params"];
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        match msg["method"].as_str() {
            Some("initialize") => {
                let edits = &params["capabilities"]["workspace"]["workspaceEdit"];
                self.snippets = edits["snippetEditSupport"] == true;
                self.reply(
                    msg,
                    json!({
                        "capabilities": {"textDocumentSync": {"openClose": true, "change": 2}},
                        "serverInfo": {"name": "parinfer-lsp"},
                    }),
                )
            }
            Some("shutdown") => self.reply(msg, Value::Null),
            Some("textDocument/didOpen") => self.open(uri, &params["textDocument"]),
            Some("textDocument/didChange") => self.change(&uri, params),
            Some("textDocument/didClose") => {
                self.docs.remove(&uri);
            }
            Some(_) if msg.get("id").is_some() => self.out.push(json!({
                "jsonrpc": "2.0", "id": msg["id"],
                "error": {"code": -32601, "message": "not supported"},
            })),
            Some(_) => {}
            None => self.answered(msg),
        }
        std::mem::take(&mut self.out)
    }

    fn reply(&mut self, msg: &Value, result: Value) {
        self.out
            .push(json!({"jsonrpc": "2.0", "id": msg["id"], "result": result}));
    }

    fn open(&mut self, uri: String, doc: &Value) {
        let text = doc["text"].as_str().unwrap_or_default().to_string();
        let language = doc["languageId"].as_str().unwrap_or_default().to_string();
        let off = trouble(&text, &language);
        if let Some(why) = &off {
            let file = file_name(&uri);
            self.out.push(show(format!(
                "Parinfer is off for {file}: {why}. It turns on once that's fixed."
            )));
        }
        let version = doc["version"].as_i64().unwrap_or(0);
        let doc = Doc {
            text,
            version,
            language,
            off: off.is_some(),
            ..Doc::default()
        };
        self.docs.insert(uri, doc);
    }

    fn change(&mut self, uri: &str, params: &Value) {
        let Some(doc) = self.docs.get_mut(uri) else {
            return;
        };
        doc.version = params["textDocument"]["version"].as_i64().unwrap_or(0);
        let before = doc.text.clone();
        let mut changes = vec![];
        for change in params["contentChanges"].as_array().into_iter().flatten() {
            let (change, was, is) = apply(&mut doc.text, change);
            (doc.prev_cursor, doc.cursor) = (was, is);
            changes.push(change);
        }
        if doc.off {
            doc.off = trouble(&doc.text, &doc.language).is_some();
            if !doc.off {
                self.out
                    .push(show(format!("Parinfer is on for {}.", file_name(uri))));
            }
        } else if doc.fix.as_ref() == Some(&doc.text) {
            doc.fix = None; // Zed echoing our fix
        } else if doc.waiting {
            doc.typed_while_waiting = true;
        } else if doc.unfixed.as_ref() == Some(&doc.text) {
            doc.unfixed = None; // undone: leave it that way
        } else {
            let changes = for_parinfer(&before, &doc.text, &changes);
            self.fix(uri, changes);
        }
    }

    fn fix(&mut self, uri: &str, changes: Vec<Value>) {
        let Some(doc) = self.docs.get_mut(uri) else {
            return;
        };
        let Some((fixed, cursor)) = smart(doc, changes) else {
            return;
        };
        if fixed == doc.text {
            return;
        }
        eprintln!("fixing version {}", doc.version);
        let snippet = cursor
            .filter(|_| self.snippets)
            .and_then(|c| snippet(&doc.text, &fixed, c));
        self.next_id += 1;
        self.requests.insert(self.next_id, uri.to_string());
        // The version lets Zed move our edits past anything typed since.
        self.out.push(json!({
            "jsonrpc": "2.0", "id": self.next_id, "method": "workspace/applyEdit",
            "params": {"label": "parinfer", "edit": {"documentChanges": [{
                "textDocument": {"uri": uri, "version": doc.version},
                "edits": snippet.unwrap_or_else(|| edits(&doc.text, &fixed)),
            }]}},
        }));
        doc.unfixed = Some(doc.text.clone());
        doc.fix = Some(fixed);
        doc.waiting = true;
    }

    fn answered(&mut self, msg: &Value) {
        let Some(uri) = msg["id"].as_i64().and_then(|id| self.requests.remove(&id)) else {
            return;
        };
        let Some(doc) = self.docs.get_mut(&uri) else {
            return;
        };
        doc.waiting = false;
        if msg["result"]["applied"] != true {
            doc.fix = None;
            doc.unfixed = None;
        }
        if std::mem::take(&mut doc.typed_while_waiting) {
            self.fix(&uri, vec![]);
        }
    }
}

// Zed says exactly where each change was, which beats parinfer working it out
// from the old text: two spaces added before two others look added after them.
fn smart(doc: &Doc, changes: Vec<Value>) -> Option<(String, Option<Pos>)> {
    let (line, x) = doc.cursor;
    let (prev_line, prev_x) = doc.prev_cursor;
    run(
        "smart",
        &doc.text,
        &doc.language,
        json!({
            "cursorLine": line, "cursorX": x,
            "prevCursorLine": prev_line, "prevCursorX": prev_x,
            "changes": changes,
        }),
    )
}

/// Why parinfer has to stay off for this text, if it does.
fn trouble(text: &str, language: &str) -> Option<String> {
    let tab = |line: &str| {
        line.chars()
            .take_while(|c| c.is_whitespace())
            .any(|c| c == '\t')
    };
    if let Some(i) = text.lines().position(tab) {
        let line = i + 1;
        return Some(format!(
            "line {line} is indented with a tab, which parinfer can't measure"
        ));
    }
    // Indent mode moves parens to match the indentation. Tidying spaces, as in
    // `(foo )`, doesn't count: it doesn't change the code.
    let (indented, _) = run("indent", text, language, json!({}))?;
    let code = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    if code(&indented) == code(text) {
        return None;
    }
    let same = text
        .lines()
        .zip(indented.lines())
        .take_while(|(a, b)| code(a) == code(b));
    let line = same.count() + 1;
    Some(format!(
        "at line {line} the indentation disagrees with the parens, and parinfer would move them"
    ))
}

// Parinfer's text, and where it says the cursor goes.
fn run(mode: &str, text: &str, language: &str, options: Value) -> Option<(String, Option<Pos>)> {
    let mut all = dialect(language);
    all.as_object_mut()?.extend(options.as_object()?.clone());
    let request: Request =
        serde_json::from_value(json!({"mode": mode, "text": text, "options": all})).ok()?;
    let answer = parinfer::process(&request);
    let cursor = answer.cursor_line.zip(answer.cursor_x);
    answer.success.then(|| (answer.text.into_owned(), cursor))
}

// How each dialect writes comments, strings and symbols. Zed's language id is
// the language's name in lower case. The rest read like Clojure.
fn dialect(language: &str) -> Value {
    let scheme = json!({"lispVlineSymbols": true, "lispBlockComments": true,
                        "schemeSexpComments": true});
    match language {
        "janet" => json!({"commentChar": "#", "janetLongStrings": true}),
        "common lisp" => json!({"lispVlineSymbols": true, "lispBlockComments": true}),
        "racket" | "scheme" => scheme,
        "guile" => {
            let mut guile = scheme;
            guile["guileBlockComments"] = json!(true);
            guile
        }
        _ => json!({}),
    }
}

struct Change {
    at: usize, // byte offset
    pos: Pos,
    old: String,
    new: String,
}

// Applies one change from Zed, and says where the cursor was and is: a
// deletion starts from its end, the way Backspace does.
fn apply(text: &mut String, change: &Value) -> (Change, Pos, Pos) {
    let new = change["text"].as_str().unwrap_or_default().to_string();
    let (at, end) = match change.get("range") {
        Some(range) => (offset(text, &range["start"]), offset(text, &range["end"])),
        None => (0, text.len()),
    };
    let pos = position(text, at);
    let was = if new.is_empty() {
        position(text, end)
    } else {
        pos
    };
    let old = text[at..end].to_string();
    text.replace_range(at..end, &new);
    let is = position(text, at + new.len());
    (Change { at, pos, old, new }, was, is)
}

// Zed sends changes top to bottom, each positioned after the ones before it,
// which is how parinfer takes them. But parinfer adds up the shifts of every
// change on a line, which goes wrong when several changes join or split lines
// (its "multi-change bug", as with `3J`): those go as one change covering all.
fn for_parinfer(before: &str, after: &str, changes: &[Change]) -> Vec<Value> {
    let lines = |c: &Change| c.old.contains('\n') || c.new.contains('\n');
    let top_down = changes
        .windows(2)
        .all(|w| w[0].at + w[0].new.len() <= w[1].at);
    match changes {
        [first, .., last] if top_down && changes.iter().any(lines) => {
            let end = last.at + last.new.len();
            let old_end = before.len() - (after.len() - end);
            vec![change(
                first.pos,
                &before[first.at..old_end],
                &after[first.at..end],
            )]
        }
        _ => changes
            .iter()
            .map(|c| change(c.pos, &c.old, &c.new))
            .collect(),
    }
}

fn change((line, x): Pos, old: &str, new: &str) -> Value {
    json!({"lineNo": line, "x": x, "oldText": old, "newText": new})
}

// LSP counts columns in UTF-16 units.
fn offset(text: &str, pos: &Value) -> usize {
    let line = pos["line"].as_u64().unwrap_or(0) as usize;
    let column = pos["character"].as_u64().unwrap_or(0) as usize;
    let start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
    let mut units = 0;
    for (i, c) in text[start..].char_indices() {
        if units >= column || c == '\n' {
            return start + i;
        }
        units += c.len_utf16();
    }
    text.len()
}

// Parinfer counts columns as they look: `１` takes two, and `å` one however
// it's written.
fn position(text: &str, offset: usize) -> Pos {
    let before = &text[..offset];
    let line_start = before.rfind('\n').map_or(0, |i| i + 1);
    let column = before[line_start..]
        .graphemes(true)
        .map(UnicodeWidthStr::width)
        .sum();
    (before.matches('\n').count(), column)
}

// The smallest edits from one text to the other, so that the text around the
// cursor stays put. Positions are in the old text.
fn edits(old: &str, new: &str) -> Vec<Value> {
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();
    let mut edits = vec![];
    for (from, to) in diff(&old, &new) {
        let text: String = new[to].iter().collect();
        edits.push(json!({"range": lsp_range(&old, from), "newText": text}));
    }
    edits
}

// Zed pushes the cursor past text added right at it, as when parinfer closes
// a paren you just opened. So the part of a fix on the cursor's line goes as
// a snippet instead, whose `$0` puts the cursor where parinfer says. Zed
// places a snippet in the text as it is by then and re-indents its lines, so
// this needs a fix that adds or removes no line breaks, which leaves every
// line where it was, and a cursor line changed only past its indentation.
fn snippet(old: &str, new: &str, cursor: Pos) -> Option<Vec<Value>> {
    let old: Vec<char> = old.chars().collect();
    let new: Vec<char> = new.chars().collect();
    let spans = diff(&old, &new);
    let breaks = |chars: &[char]| chars.contains(&'\n');
    if spans
        .iter()
        .any(|(o, n)| breaks(&old[o.clone()]) || breaks(&new[n.clone()]))
    {
        return None;
    }
    let at = char_index(&new, cursor);
    let line = new[..at].iter().filter(|&&c| c == '\n').count();
    let line_of = |i: usize| old[..i].iter().filter(|&&c| c == '\n').count();
    let (here, elsewhere): (Vec<_>, Vec<_>) = spans
        .into_iter()
        .partition(|(o, _)| line_of(o.start) == line);
    let (from, to) = (here.first()?.0.start, here.last()?.0.end);
    let (new_from, new_to) = (here.first()?.1.start, here.last()?.1.end);
    let line_start = old[..from]
        .iter()
        .rposition(|&c| c == '\n')
        .map_or(0, |i| i + 1);
    let past_indent = old[line_start..from].iter().any(|c| !c.is_whitespace());
    if !past_indent || !(new_from..=new_to).contains(&at) {
        return None;
    }
    let escape = |chars: &[char]| {
        let mut text = String::new();
        for &c in chars {
            if matches!(c, '\\' | '$' | '}') {
                text.push('\\');
            }
            text.push(c);
        }
        text
    };
    let value = format!(
        "{}$0{}",
        escape(&new[new_from..at]),
        escape(&new[at..new_to])
    );
    let snippet = json!({"kind": "snippet", "value": value});
    let mut edits = vec![(
        from,
        json!({"range": lsp_range(&old, from..to), "snippet": snippet}),
    )];
    for (o, n) in elsewhere {
        let text: String = new[n].iter().collect();
        edits.push((
            o.start,
            json!({"range": lsp_range(&old, o), "newText": text}),
        ));
    }
    edits.sort_by_key(|(start, _)| *start);
    Some(edits.into_iter().map(|(_, edit)| edit).collect())
}

fn diff(old: &[char], new: &[char]) -> Vec<(Range<usize>, Range<usize>)> {
    capture_diff_slices(Algorithm::Myers, old, new)
        .iter()
        .map(|op| op.as_tag_tuple())
        .filter(|(tag, _, _)| *tag != DiffTag::Equal)
        .map(|(_, from, to)| (from, to))
        .collect()
}

// Where parinfer's (line, column) falls in the text, as a character index.
fn char_index(text: &[char], (line, x): Pos) -> usize {
    let mut i = 0;
    for _ in 0..line {
        i += text[i..]
            .iter()
            .position(|&c| c == '\n')
            .map_or(text.len() - i, |p| p + 1);
    }
    let mut column = 0;
    while i < text.len() && text[i] != '\n' && column < x {
        column += text[i].width().unwrap_or(0);
        i += 1;
    }
    i
}

fn lsp_range(text: &[char], range: Range<usize>) -> Value {
    json!({"start": lsp_position(text, range.start), "end": lsp_position(text, range.end)})
}

fn lsp_position(text: &[char], offset: usize) -> Value {
    let before = &text[..offset];
    let line_start = before.iter().rposition(|&c| c == '\n').map_or(0, |i| i + 1);
    let column: usize = before[line_start..].iter().map(|c| c.len_utf16()).sum();
    json!({"line": before.iter().filter(|&&c| c == '\n').count(), "character": column})
}

fn show(message: String) -> Value {
    eprintln!("{message}");
    json!({"jsonrpc": "2.0", "method": "window/showMessage",
           "params": {"type": 3, "message": message}})
}

fn file_name(uri: &str) -> &str {
    uri.rsplit('/').next().unwrap_or(uri)
}

/// Reads one message: headers, a blank line, then a JSON body.
pub fn read(input: &mut impl BufRead) -> Option<Value> {
    let mut length = 0;
    loop {
        let mut header = String::new();
        if input.read_line(&mut header).ok()? == 0 {
            return None;
        }
        match header.trim_end().split_once(": ") {
            Some(("Content-Length", n)) => length = n.parse().ok()?,
            None if header.trim_end().is_empty() => break,
            _ => {}
        }
    }
    let mut body = vec![0; length];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

pub fn write(output: &mut impl Write, msg: &Value) -> std::io::Result<()> {
    let body = msg.to_string();
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}
