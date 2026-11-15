//! A listing of the values of a document by address, for a person to read.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::format::{MAX_DEPTH, ext};
use crate::reader::{Doc, Raw};

/// One line per value reachable from the root, in the order of their positions: the
/// position, then the value, or `&` and the position a pointer leads to. The items of a
/// collection follow it, indented.
///
/// The bytes need not be a checked document: what is not a value shows as `?`.
pub fn dump(doc: &Doc<'_>) -> String {
    let mut dump = Dump { doc, lines: BTreeMap::new() };
    if let Some(end) = doc.end().checked_sub(2) {
        dump.slot(end, false, 0, 0);
    }
    let mut text = String::new();
    for (pos, line) in dump.lines {
        let _ = writeln!(text, "{pos:04x}: {line}");
    }
    text
}

struct Dump<'d, 'a> {
    doc: &'d Doc<'a>,
    lines: BTreeMap<usize, String>,
}

impl Dump<'_, '_> {
    /// Lists the slot at `addr`: `indent` is how far its line is set in, `depth` how many
    /// values lead to it.
    fn slot(&mut self, addr: usize, wide: bool, indent: usize, depth: usize) {
        if !self.doc.is_pointer(addr) {
            return self.value(addr, indent, depth);
        }
        let margin = "  ".repeat(indent);
        let Some((target, _)) = self.doc.pointer(addr, wide) else {
            self.lines.insert(addr, format!("{margin}&?"));
            return;
        };
        self.lines.insert(addr, format!("{margin}&{target:04x}"));
        if self.doc.is_pointer(target) {
            // Only the pointer that ends a document leads to another pointer, a wide one.
            self.slot(target, true, indent, depth + 1);
        } else {
            self.value(target, 0, depth + 1);
        }
    }

    fn value(&mut self, pos: usize, indent: usize, depth: usize) {
        if self.lines.contains_key(&pos) || depth > 4 * MAX_DEPTH {
            return;
        }
        let raw = self.doc.raw(pos).map(|raw| raw.0);
        let text = match raw {
            None | Some(Raw::Pointer) => "?".to_owned(),
            Some(Raw::Int(v)) => v.to_string(),
            Some(Raw::UInt(v)) => v.to_string(),
            Some(Raw::F32(v)) => format!("{v:?}f32"),
            Some(Raw::F64(v)) => format!("{v:?}"),
            Some(Raw::Null) => "null".to_owned(),
            Some(Raw::Undefined) => "undefined".to_owned(),
            Some(Raw::Bool(v)) => v.to_string(),
            Some(Raw::Str(bytes)) => format!("{:?}", String::from_utf8_lossy(bytes)),
            Some(Raw::Data(bytes)) => format!("data[{}]", bytes.len()),
            Some(Raw::Ext(kind)) => format!("ext {}", ext::name(kind).unwrap_or("?")),
            Some(Raw::Coll { dict, wide, count, .. }) => {
                let name = if dict { "Dict" } else { "Array" };
                format!("{name}[{count}]{}", if wide { " wide" } else { "" })
            }
        };
        self.lines.insert(pos, format!("{}{text}", "  ".repeat(indent)));
        match raw {
            Some(Raw::Ext(kind)) if kind != ext::NONE => {
                self.value(pos + 2, indent + 1, depth + 1);
            }
            Some(Raw::Coll { dict, wide, count, first }) => {
                let width = if wide { 4 } else { 2 };
                for i in 0..if dict { 2 * count } else { count } {
                    self.slot(first + i * width, wide, indent + 1, depth + 1);
                }
            }
            _ => {}
        }
    }
}
