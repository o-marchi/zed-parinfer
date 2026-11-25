mod common;
use common::Zed;

// Dedent the last line out of the form: the paren must close before it,
// whatever parens the comment or symbol in the middle holds.
fn dedent_last_line(language: &str, text: &str) -> String {
    let mut zed = Zed::open(language, text);
    zed.replace((2, 0), (2, 2), "");
    zed.text
}

#[test]
fn clojure_line_comments() {
    let text = "(defn f [x]\n  ; a (comment\n  (+ x 1))\n";
    assert_eq!(
        dedent_last_line("clojure", text),
        "(defn f [x])\n  ; a (comment\n(+ x 1)\n"
    );
}

#[test]
fn janet_comments_start_with_a_hash() {
    let text = "(defn f [x]\n  # a (comment\n  (+ x 1))\n";
    assert_eq!(
        dedent_last_line("janet", text),
        "(defn f [x])\n  # a (comment\n(+ x 1)\n"
    );
}

#[test]
fn janet_long_strings() {
    let text = "(def s\n  ``a (string``\n  (+ 1 2))\n";
    assert_eq!(
        dedent_last_line("janet", text),
        "(def s\n  ``a (string``)\n(+ 1 2)\n"
    );
}

#[test]
fn racket_block_comments() {
    let text = "(define (f x)\n  #| a (block |#\n  (+ x 1))\n";
    assert_eq!(
        dedent_last_line("racket", text),
        "(define (f x)\n  #| a (block |#)\n(+ x 1)\n"
    );
}

#[test]
fn scheme_symbols_between_bars() {
    let text = "(define (f x)\n  '|a (symbol|\n  (+ x 1))\n";
    assert_eq!(
        dedent_last_line("scheme", text),
        "(define (f x)\n  '|a (symbol|)\n(+ x 1)\n"
    );
}

#[test]
fn common_lisp_block_comments_and_symbols() {
    let text = "(defun f (x)\n  #| a (block |# '|a (sym|\n  (+ x 1))\n";
    let fixed = "(defun f (x)\n  #| a (block |# '|a (sym|)\n(+ x 1)\n";
    assert_eq!(dedent_last_line("common lisp", text), fixed);
}

#[test]
fn guile_block_comments() {
    let text = "(define (f x)\n  #! a (block !#\n  (+ x 1))\n";
    assert_eq!(
        dedent_last_line("guile", text),
        "(define (f x)\n  #! a (block !#)\n(+ x 1)\n"
    );
}

#[test]
fn the_language_decides() {
    // Read as Clojure, the `#` line is code, and its paren counts.
    let text = "(defn f [x]\n  # a (comment\n  (+ x 1))\n";
    assert_ne!(
        dedent_last_line("clojure", text),
        dedent_last_line("janet", text)
    );
}

#[test]
fn racket_block_comments_nest() {
    let text = "(define (f x)\n  #| a #| (nested |# (block |#\n  (+ x 1))\n";
    let fixed = "(define (f x)\n  #| a #| (nested |# (block |#)\n(+ x 1)\n";
    assert_eq!(dedent_last_line("racket", text), fixed);
}

#[test]
fn a_scheme_datum_comment_still_holds_a_form() {
    // `#;` comments out the next form, but that form's parens are still parens.
    let text = "(define (f x)\n  #;(g x)\n  (+ x 1))\n";
    assert_eq!(
        dedent_last_line("scheme", text),
        "(define (f x)\n  #;(g x))\n(+ x 1)\n"
    );
}

#[test]
fn a_close_paren_inside_bars_is_part_of_the_symbol() {
    let text = "(define (f x)\n  '|a )|\n  (+ x 1))\n";
    assert_eq!(
        dedent_last_line("scheme", text),
        "(define (f x)\n  '|a )|)\n(+ x 1)\n"
    );
}

#[test]
fn clojure_reader_macros_are_code() {
    let text = "(defn f [x]\n  #(inc %) #{1 2}\n  (+ x 1))\n";
    assert_eq!(
        dedent_last_line("clojure", text),
        "(defn f [x]\n  #(inc %) #{1 2})\n(+ x 1)\n"
    );
}
