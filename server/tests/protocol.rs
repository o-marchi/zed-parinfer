use parinfer_lsp::{read, write};
use serde_json::json;

#[test]
fn reads_a_message_after_its_headers() {
    let mut input = "Content-Length: 13\r\nContent-Type: x\r\n\r\n{\"id\": 1}    ".as_bytes();
    assert_eq!(read(&mut input), Some(json!({"id": 1})));
}

#[test]
fn reads_one_message_at_a_time() {
    let mut input = "Content-Length: 2\r\n\r\n{}Content-Length: 4\r\n\r\nnull".as_bytes();
    assert_eq!(read(&mut input), Some(json!({})));
    assert_eq!(read(&mut input), Some(json!(null)));
    assert_eq!(read(&mut input), None);
}

#[test]
fn stops_at_the_end_of_input() {
    assert_eq!(read(&mut "".as_bytes()), None);
    assert_eq!(read(&mut "Content-Length: 10\r\n\r\n{}".as_bytes()), None);
}

#[test]
fn writes_the_length_in_bytes() {
    let mut output = vec![];
    write(&mut output, &json!("é")).unwrap();
    assert_eq!(
        String::from_utf8(output).unwrap(),
        "Content-Length: 4\r\n\r\n\"é\""
    );
}
