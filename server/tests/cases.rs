//! Parinfer's own test cases, sent to the server the way Zed would. They come
//! from parinfer-rust (ISC), which has them from parinfer.js (MIT); both
//! licences are in cases/.

mod common;
use common::{At, Zed};
use serde_json::Value;

struct Change {
    from: At,
    to: At,
    old: String,
    new: String,
}

fn cases(json: &str) -> Vec<Value> {
    serde_json::from_str(json).unwrap()
}

// A case's changes are positioned in the text after them. Applied in order,
// each one's position also holds in the text so far, as with Zed.
fn changes(case: &Value) -> Vec<Change> {
    let changes = case["options"]["changes"].as_array().into_iter().flatten();
    changes
        .map(|change| {
            let from = (
                change["lineNo"].as_u64().unwrap() as usize,
                change["x"].as_u64().unwrap() as usize,
            );
            let old = change["oldText"].as_str().unwrap().to_string();
            let to = match old.rsplit_once('\n') {
                Some((before, last)) => (from.0 + before.matches('\n').count() + 1, last.len()),
                None => (from.0, from.1 + old.len()),
            };
            Change {
                from,
                to,
                old,
                new: change["newText"].as_str().unwrap().to_string(),
            }
        })
        .collect()
}

// The text before the changes: undo them, last first.
fn before(case: &Value) -> String {
    let mut text = case["text"].as_str().unwrap().to_string();
    for change in changes(case).into_iter().rev() {
        let (line, x) = change.from;
        let line_start: usize = text.split_inclusive('\n').take(line).map(str::len).sum();
        let start = line_start + x;
        text.replace_range(start..start + change.new.len(), &change.old);
    }
    text
}

// These give no cursor, and their answer depends on it. Zed's would be where
// the change ended, and parinfer leaves the parens after it where they are.
const NEED_THEIR_CURSOR: [u64; 2] = [208, 238];

#[test]
fn smart_mode() {
    let mut checked = 0;
    for case in cases(include_str!("cases/smart-mode.json")) {
        let line = case["source"]["lineNo"].as_u64().unwrap();
        let steps = case["source"]["in"].as_array().unwrap().len();
        let changes = changes(&case);
        // Several steps, or a cursor move alone, aren't anything Zed sends.
        if steps > 1 || changes.is_empty() || NEED_THEIR_CURSOR.contains(&line) {
            continue;
        }
        let mut zed = Zed::open("clojure", &before(&case));
        let sent: Vec<_> = changes
            .iter()
            .map(|c| (c.from, c.to, c.new.as_str()))
            .collect();
        zed.change(&sent);
        if case["result"]["success"] == true {
            assert_eq!(
                zed.text,
                case["result"]["text"].as_str().unwrap(),
                "case at line {line}"
            );
            assert!(
                zed.fixes <= 1,
                "case at line {line}: a fix must not need fixing"
            );
        } else {
            assert_eq!(zed.fixes, 0, "case at line {line} fails, so nothing to fix");
        }
        checked += 1;
    }
    assert_eq!(checked, 16);
}

#[test]
fn indent_mode_decides_when_to_stay_off() {
    let code = |s: &str| s.chars().filter(|c| !c.is_whitespace()).collect::<String>();
    for case in cases(include_str!("cases/indent-mode.json")) {
        let text = case["text"].as_str().unwrap();
        let moves_parens = case["result"]["success"] == true
            && code(case["result"]["text"].as_str().unwrap()) != code(text);
        let zed = Zed::open("clojure", text);
        let line = case["source"]["lineNo"].as_u64().unwrap();
        assert_eq!(
            !zed.messages.is_empty(),
            moves_parens,
            "case at line {line}"
        );
    }
}
