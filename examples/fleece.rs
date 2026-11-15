//! A tool over the crate, after the `fleece` tool of couchbase/fleece.
//!
//! ```text
//! fleece encode|decode|dump       < input > output
//! fleece diff OLD.json NEW.json   > delta.json
//! fleece apply OLD.json DELTA.json > new.json
//! fleece append BASE.fleece NEW.json > delta.fleece
//! fleece compact                  < document > document
//! ```

use std::io::{Read, Write};

use fleece::json::{from_json, to_json};
use fleece::{Doc, Value};

type Error = Box<dyn std::error::Error>;

fn json_file(path: &str) -> Result<Value, Error> {
    Ok(from_json(&serde_json::from_str(&std::fs::read_to_string(path)?)?)?)
}

fn main() -> Result<(), Error> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let stdin = || -> Result<Vec<u8>, Error> {
        let mut input = Vec::new();
        std::io::stdin().read_to_end(&mut input)?;
        Ok(input)
    };
    let output = match args[..] {
        ["encode"] => fleece::json::json_to_fleece(std::str::from_utf8(&stdin()?)?)?,
        ["decode"] => fleece::json::fleece_to_json(&stdin()?)?.into_bytes(),
        ["dump"] => fleece::dump(&Doc::new(&stdin()?)?).into_bytes(),
        ["compact"] => fleece::compact(&Doc::new(&stdin()?)?)?,
        ["diff", old, new] => {
            let delta = fleece::delta::diff(&json_file(old)?, &json_file(new)?);
            // Fleece gives an empty object for two values that are the same.
            to_json(&delta.unwrap_or(Value::Object(Default::default()))).to_string().into_bytes()
        }
        ["apply", old, delta] => {
            let new = fleece::delta::apply(&json_file(old)?, &json_file(delta)?)?;
            to_json(&new).to_string().into_bytes()
        }
        ["append", base, new] => {
            fleece::delta::append_delta(&std::fs::read(base)?, &json_file(new)?)?
        }
        _ => return Err("usage: fleece encode|decode|dump|compact|diff|apply|append".into()),
    };
    std::io::stdout().write_all(&output)?;
    Ok(())
}
