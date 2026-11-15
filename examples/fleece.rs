//! `fleece encode|decode|dump`: JSON on the standard input to a document on the standard
//! output, a document back to JSON, or a document to its listing by address.

use std::io::{Read, Write};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = std::env::args().nth(1).unwrap_or_default();
    let mut input = Vec::new();
    std::io::stdin().read_to_end(&mut input)?;
    let output = match mode.as_str() {
        "encode" => fleece::json::json_to_fleece(std::str::from_utf8(&input)?)?,
        "decode" => fleece::json::fleece_to_json(&input)?.into_bytes(),
        "dump" => fleece::dump(&fleece::Doc::new(&input)?).into_bytes(),
        _ => return Err("usage: fleece encode|decode|dump < input > output".into()),
    };
    std::io::stdout().write_all(&output)?;
    Ok(())
}
