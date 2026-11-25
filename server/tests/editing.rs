mod common;
use common::Zed;
use serde_json::json;

#[test]
fn asks_for_incremental_changes() {
    let mut zed = Zed::open("clojure", "");
    let reply = zed.send(json!({"id": 1, "method": "initialize", "params": {}}));
    assert_eq!(
        reply[0]["result"]["capabilities"]["textDocumentSync"]["change"],
        2
    );
}

#[test]
fn indenting_a_form_moves_it_inside_the_one_above() {
    let mut zed = Zed::open("clojure", "(when ready?)\n(run)\n");
    zed.type_at((1, 0), "  ");
    assert_eq!(zed.text, "(when ready?\n  (run))\n");
}

#[test]
fn dedenting_moves_it_back_out() {
    let mut zed = Zed::open("clojure", "(when ready?\n  (run))\n");
    zed.replace((1, 0), (1, 2), "");
    assert_eq!(zed.text, "(when ready?)\n(run)\n");
}

#[test]
fn a_form_brings_its_lines_along() {
    let mut zed = Zed::open("clojure", "(defn f [x])\n(let [y 1]\n  (+ x y))\n");
    zed.type_at((1, 0), "  ");
    assert_eq!(zed.text, "(defn f [x]\n  (let [y 1]\n    (+ x y)))\n");
}

#[test]
fn indenting_several_lines_at_once() {
    let mut zed = Zed::open("clojure", "(when ready?)\n(run)\n(stop)\n");
    zed.change(&[((1, 0), (1, 0), "  "), ((2, 0), (2, 0), "  ")]);
    assert_eq!(zed.text, "(when ready?\n  (run)\n  (stop))\n");
}

#[test]
fn changes_may_come_bottom_up() {
    // Zed sends them top down, but LSP allows any order.
    let mut zed = Zed::open("clojure", "(when ready?)\n(run)\n(stop)\n");
    zed.change(&[((2, 0), (2, 0), "  "), ((1, 0), (1, 0), "  ")]);
    assert_eq!(zed.text, "(when ready?\n  (run)\n  (stop))\n");
}

#[test]
fn typing_inside_a_form_needs_no_fix() {
    let mut zed = Zed::open("clojure", "(foo bar)\n");
    zed.type_at((0, 8), " baz");
    assert_eq!((zed.text.as_str(), zed.fixes), ("(foo bar baz)\n", 0));
}

#[test]
fn an_open_paren_gets_closed() {
    let mut zed = Zed::open("clojure", "(foo )\n");
    zed.type_at((0, 5), "(");
    assert_eq!(zed.text, "(foo ())\n");
}

#[test]
fn typing_a_close_paren_ends_the_form_there() {
    let mut zed = Zed::open("clojure", "(foo bar)\n");
    zed.type_at((0, 4), ")");
    assert_eq!(zed.text, "(foo) bar\n");
}

#[test]
fn deleting_an_open_paren_takes_its_close_paren() {
    let mut zed = Zed::open("clojure", "(foo (bar))\n");
    zed.replace((0, 5), (0, 6), "");
    assert_eq!(zed.text, "(foo bar)\n");
}

#[test]
fn its_own_fix_is_not_fixed_again() {
    let mut zed = Zed::open("clojure", "(when ready?)\n(run)\n");
    zed.type_at((1, 0), "  ");
    assert_eq!(zed.fixes, 1);
}

#[test]
fn an_undone_fix_stays_undone() {
    let mut zed = Zed::open("clojure", "(when ready?)\n(run)\n");
    zed.type_at((1, 0), "  ");
    zed.undo();
    assert_eq!(
        (zed.text.as_str(), zed.fixes),
        ("(when ready?)\n  (run)\n", 1)
    );
}

#[test]
fn typing_while_a_fix_is_on_its_way_is_fixed_after() {
    let mut zed = Zed::open("clojure", "(a)\n(b)\n(c)\n");
    zed.hold = true;
    zed.type_at((1, 0), "  ");
    zed.type_at((2, 0), "  ");
    zed.hold = false;
    zed.release(true);
    assert_eq!(zed.text, "(a\n  (b)\n  (c))\n");
}

#[test]
fn a_refused_fix_is_forgotten() {
    let mut zed = Zed::open("clojure", "(a)\n(b)\n");
    zed.hold = true;
    zed.type_at((1, 0), "  ");
    zed.hold = false;
    zed.release(false);
    zed.type_at((1, 2), "x");
    assert_eq!(zed.text, "(a\n  x(b))\n");
}

#[test]
fn an_unclosed_string_waits_until_it_is_closed() {
    let mut zed = Zed::open("clojure", "(foo)\n");
    zed.type_at((0, 4), " \"");
    assert_eq!((zed.text.as_str(), zed.fixes), ("(foo \")\n", 0));
}

#[test]
fn stays_off_where_indentation_and_parens_disagree() {
    let mut zed = Zed::open("clojure", "(foo\nbar)\n(baz)\n");
    assert!(zed.messages[0].contains("off") && zed.messages[0].contains("line 1"));
    zed.type_at((2, 4), " qux");
    assert_eq!(
        (zed.text.as_str(), zed.fixes),
        ("(foo\nbar)\n(baz qux)\n", 0)
    );
}

#[test]
fn a_space_before_a_close_paren_is_no_disagreement() {
    let zed = Zed::open("clojure", "(foo )\n");
    assert!(zed.messages.is_empty());
}

#[test]
fn turns_on_once_they_agree() {
    let mut zed = Zed::open("clojure", "(foo\nbar)\n(baz)\n");
    zed.type_at((1, 0), " ");
    assert_eq!(zed.messages[1], "Parinfer is on for test.");
    zed.type_at((2, 0), "  ");
    assert_eq!(zed.text, "(foo\n bar\n  (baz))\n");
}

#[test]
fn columns_count_utf16_units() {
    // The emoji is two UTF-16 units: get that wrong and the fix lands off.
    let mut zed = Zed::open("clojure", "(def s \"😀\")\n(b)\n");
    zed.type_at((0, 11), " x");
    zed.type_at((1, 0), "  ");
    assert_eq!(zed.text, "(def s \"😀\" x\n  (b))\n");
}

#[test]
fn a_closed_document_is_forgotten() {
    let mut zed = Zed::open("clojure", "(a)\n(b)\n");
    zed.send(json!({"method": "textDocument/didClose",
                    "params": {"textDocument": {"uri": "file:///test"}}}));
    zed.type_at((1, 0), "  ");
    assert_eq!(zed.fixes, 0);
}

#[test]
fn unknown_requests_get_an_error_and_notifications_nothing() {
    let mut zed = Zed::open("clojure", "");
    let reply = zed.send(json!({"id": 7, "method": "textDocument/hover", "params": {}}));
    assert_eq!(
        (&reply[0]["id"], &reply[0]["error"]["code"]),
        (&json!(7), &json!(-32601))
    );
    assert!(zed
        .send(json!({"method": "$/cancelRequest", "params": {}}))
        .is_empty());
}

#[test]
fn shutdown_answers_null() {
    let mut zed = Zed::open("clojure", "");
    let reply = zed.send(json!({"id": 9, "method": "shutdown"}));
    assert_eq!(reply[0]["result"], json!(null));
}

// From parinfer-rust's Vim scenarios and parinfer-rust-mode's user cases.

#[test]
fn an_open_bracket_pushed_right_takes_its_lines_along() {
    let mut zed = Zed::open("clojure", "(foo [\n      x])\n");
    zed.type_at((0, 2), "b");
    assert_eq!(zed.text, "(fboo [\n       x])\n");
}

#[test]
fn changing_a_word_keeps_the_lines_below_aligned() {
    let mut zed = Zed::open("clojure", "(foo [\n      x])\n");
    zed.replace((0, 1), (0, 4), "");
    zed.type_at((0, 1), "a");
    assert_eq!(zed.text, "(a [\n    x])\n");
}

#[test]
fn indenting_a_line_brings_the_rest_of_its_form() {
    let mut zed = Zed::open("clojure", "(foo\n  (bar\n    baz\n    quux))\n");
    zed.type_at((1, 0), "  ");
    assert_eq!(zed.text, "(foo\n    (bar\n      baz\n      quux))\n");
}

#[test]
fn appending_after_an_open_bracket() {
    let mut zed = Zed::open("clojure", "(let [\n      x])\n");
    zed.type_at((0, 6), "y");
    assert_eq!(zed.text, "(let [y\n      x])\n");
}

#[test]
fn replacing_words_with_forms() {
    let mut zed = Zed::open("clojure", "(foo bar\n     baz)\n");
    zed.replace((0, 5), (0, 8), "(hi");
    zed.replace((1, 5), (1, 8), "(hi");
    assert_eq!(zed.text, "(foo (hi)\n     (hi))\n");
}

#[test]
fn deleting_to_the_end_of_a_line_keeps_the_forms_closed() {
    let mut zed = Zed::open("clojure", "(let [a \"foo\"])\n");
    zed.replace((0, 8), (0, 15), "");
    assert_eq!(zed.text, "(let [a ])\n");
}

#[test]
fn joining_two_lines() {
    let mut zed = Zed::open("clojure", "(foo\n  (bar\n    baz))\n");
    zed.replace((0, 4), (1, 2), " ");
    assert_eq!(zed.text, "(foo (bar\n       baz))\n");
}

#[test]
fn joining_three_lines() {
    let mut zed = Zed::open("clojure", "(foo\n  (bar\n    [baz\n     quux]))\n");
    zed.change(&[((0, 4), (1, 2), " "), ((0, 9), (1, 4), " ")]);
    assert_eq!(zed.text, "(foo (bar [baz\n           quux]))\n");
}

#[test]
fn joining_a_map_keeps_its_keys_aligned() {
    let mut zed = Zed::open("clojure", "(def thing\n  {:abc 1\n   :xyz 2})\n");
    zed.replace((0, 10), (1, 2), " ");
    assert_eq!(zed.text, "(def thing {:abc 1\n            :xyz 2})\n");
}

#[test]
fn two_undos_take_back_a_typed_bracket() {
    let mut zed = Zed::open("clojure", "x\n\ny\n");
    zed.type_at((1, 0), "[");
    assert_eq!(zed.text, "x\n[]\ny\n");
    zed.undo();
    zed.replace((1, 0), (1, 1), "");
    assert_eq!((zed.text.as_str(), zed.fixes), ("x\n\ny\n", 1));
}

#[test]
fn opening_a_file_never_changes_it() {
    let zed = Zed::open("clojure", "{\n    }\n");
    assert_eq!((zed.text.as_str(), zed.fixes), ("{\n    }\n", 0));
}

#[test]
fn brackets_in_strings_are_text() {
    let mut zed = Zed::open("clojure", "(map! \"] E\")\n");
    zed.type_at((0, 5), "x");
    assert_eq!(
        (zed.text.as_str(), zed.messages.len()),
        ("(map!x \"] E\")\n", 0)
    );
}

#[test]
fn a_letter_written_as_two_characters_takes_one_column() {
    let decomposed = "a\u{308}a\u{308}a\u{308}"; // äää, each a plus a combining mark
    let mut zed = Zed::open("clojure", "(def aaa {:foo 1\n          :bar 1})\n");
    zed.replace((0, 5), (0, 8), decomposed);
    assert_eq!(
        zed.text,
        format!("(def {decomposed} {{:foo 1\n          :bar 1}})\n")
    );
}

#[test]
fn typing_after_a_composed_letter() {
    let mut zed = Zed::open("clojure", "(def a\u{30a} [])\n");
    zed.type_at((0, 9), "x");
    assert_eq!(zed.text, "(def a\u{30a} [x])\n");
}

#[test]
fn a_fullwidth_character_takes_two_columns() {
    // Typed after the bracket, which doesn't move, so `y` must stay put.
    let mut zed = Zed::open("clojure", "(１２ [x\n       y])\n");
    zed.type_at((0, 5), "z");
    assert_eq!(zed.text, "(１２ [zx\n       y])\n");
}

#[test]
fn a_space_after_a_wide_character_stays() {
    let mut zed = Zed::open("clojure", "(func 中)\n");
    zed.type_at((0, 7), " ");
    assert_eq!(zed.text, "(func 中 )\n");
}

#[test]
fn windows_line_endings() {
    let mut zed = Zed::open("clojure", "(when ready?)\r\n(run)\r\n");
    zed.type_at((1, 0), "  ");
    assert_eq!(zed.text, "(when ready?\r\n  (run))\r\n");
}

#[test]
fn a_file_indented_with_tabs_stays_off() {
    let zed = Zed::open("clojure", "(foo\n\t(bar))\n");
    assert_eq!(zed.messages.len(), 1);
}

#[test]
fn a_long_file_is_quick() {
    let form = "(defn f [x]\n  (let [y (* x 2)]\n    {:x x\n     :y y}))\n\n";
    let mut zed = Zed::open("clojure", &form.repeat(600));
    let start = std::time::Instant::now();
    zed.type_at((1, 0), "  ");
    assert!(
        start.elapsed().as_millis() < 500,
        "took {:?}",
        start.elapsed()
    );
    assert_eq!(zed.fixes, 1);
}

// Without auto-closing brackets, as in Common Lisp's Zed extension, Parinfer
// adds the closer right at the cursor, and Zed would push the cursor past it.

#[test]
fn an_open_paren_without_auto_closing_keeps_the_cursor_inside() {
    let mut zed = Zed::open("common lisp", "(foo)\n\n");
    zed.autoclose = false;
    zed.move_to((1, 0));
    zed.type_keys("(");
    assert_eq!((zed.text.as_str(), zed.cursor()), ("(foo)\n()\n", (1, 1)));
}

#[test]
fn typing_a_form_without_auto_closing_or_closers() {
    let mut zed = Zed::open("common lisp", "(defun f (x)\n  (g x))\n");
    zed.autoclose = false;
    zed.type_at((1, 0), "  \n");
    zed.move_to((1, 2));
    zed.type_keys("(let ((y (* x 2");
    assert_eq!(zed.text, "(defun f (x)\n  (let ((y (* x 2))))\n  (g x))\n");
}

#[test]
fn typing_nested_forms_and_their_closers_without_auto_closing() {
    let body = "(format t \"~a~%* ~a *~%~a~%\" line message line)";
    let mut zed = Zed::open(
        "common lisp",
        &format!("(defun banner (message)\n  {body})\n"),
    );
    zed.autoclose = false;
    zed.type_at((1, 0), "  \n");
    zed.move_to((1, 2));
    let line = "(let ((line (make-string (+ 4 (length message)) :initial-element #\\*)))";
    zed.type_keys(line);
    zed.type_at((2, 0), "  ");
    assert_eq!(
        zed.text,
        format!("(defun banner (message)\n  {line}\n    {body}))\n")
    );
}

#[test]
fn a_new_form_below_takes_the_closers_from_the_line_above() {
    let text = "(defun week ()\n  (dotimes (i 7)\n    (day i)))\n";
    let mut zed = Zed::open("common lisp", text);
    zed.autoclose = false;
    zed.type_at((2, 13), "\n    ");
    zed.type_keys("(print :done");
    let fixed = "(defun week ()\n  (dotimes (i 7)\n    (day i)\n    (print :done)))\n";
    assert_eq!((zed.text.as_str(), zed.cursor()), (fixed, (3, 16)));
}
