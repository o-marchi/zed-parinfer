use parinfer_lsp::{read, write, Server};
use std::io;

fn main() -> io::Result<()> {
    let mut server = Server::default();
    let (mut input, mut output) = (io::stdin().lock(), io::stdout().lock());
    while let Some(msg) = read(&mut input) {
        if msg["method"] == "exit" {
            break;
        }
        for reply in server.handle(&msg) {
            write(&mut output, &reply)?;
        }
    }
    Ok(())
}
