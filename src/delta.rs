//! The difference between two values, in two forms.
//!
//! [`append_delta`] gives bytes to append to an encoded document: the new root, with every
//! part that did not change written as a pointer into the old bytes, and a dictionary of
//! which few entries changed written as those entries over the old one. The result is read
//! like any document, without applying anything. A document that took many deltas holds
//! what they replaced; see [`crate::compact`].
//!
//! [`diff`] gives the changes as a value, in the form Fleece documents in `Deltas.md` for its
//! JSON deltas, and [`apply`] turns the old value into the new one with it. It suits sending a
//! change to a side that has the old value.

use crate::encoder::Encoder;
use crate::error::{DeltaError, EncodeError};
use crate::reader::{DictRef, Doc, Item, ValueRef};
use crate::value::{Number, Object, Value};

/// Through how many dictionaries a dictionary written by a delta may inherit. A lookup of a
/// key asks each of them in turn, so past this depth the dictionary is written whole.
const MAX_INHERIT_DEPTH: usize = 4;

/// The bytes that, appended to the document `base`, make a document whose root is `new`.
pub fn append_delta(base: &[u8], new: &Value) -> Result<Vec<u8>, DeltaError> {
    let root = Doc::new(base)?.root();
    let mut encoder = Encoder::new().with_base(base, false)?;
    encoder.write_changed(root, new)?;
    Ok(encoder.finish()?)
}

/// What to write for a new value, given the old one at the same place.
enum Plan {
    /// The old value, as a pointer to it.
    Same,
    /// The new value, whole.
    Replace,
    /// A collection of the same kind: a plan for each field or item of the new one, in its
    /// order.
    Children(Vec<Plan>),
}

/// Compares the two values once, from the leaves up.
fn plan(old: &Value, new: &Value) -> Plan {
    let children: Vec<Plan> = match (old, new) {
        (Value::Object(old), Value::Object(new)) => {
            let child = |(key, value): (&String, &Value)| {
                old.get(key).map_or(Plan::Replace, |old| plan(old, value))
            };
            let children: Vec<Plan> = new.iter().map(child).collect();
            if old.len() != new.len() {
                return Plan::Children(children);
            }
            children
        }
        (Value::Array(old), Value::Array(new)) => {
            let child = |(index, value): (usize, &Value)| {
                old.get(index).map_or(Plan::Replace, |old| plan(old, value))
            };
            let children: Vec<Plan> = new.iter().enumerate().map(child).collect();
            if old.len() != new.len() {
                return Plan::Children(children);
            }
            children
        }
        _ => return if old == new { Plan::Same } else { Plan::Replace },
    };
    if children.iter().all(|child| matches!(child, Plan::Same)) {
        Plan::Same
    } else {
        Plan::Children(children)
    }
}

impl Encoder<'_> {
    /// Writes `new`, pointing at `old` and at the parts of it that are the same in `new`.
    /// `old` has to be a value of the document given to [`Encoder::with_base`].
    pub fn write_changed(&mut self, old: ValueRef<'_>, new: &Value) -> Result<(), EncodeError> {
        match Value::from_ref(old) {
            Ok(old_value) => self.changed(old, &old_value, new, &plan(&old_value, new)),
            Err(_) => self.write_value(new),
        }
    }

    fn changed(
        &mut self,
        at: ValueRef<'_>,
        old: &Value,
        new: &Value,
        plan: &Plan,
    ) -> Result<(), EncodeError> {
        let children = match plan {
            Plan::Same => return self.write_ref(at),
            Plan::Replace => return self.write_value(new),
            Plan::Children(children) => children,
        };
        match (at.item(), old, new) {
            (Item::Dict(dict), Value::Object(old), Value::Object(new)) => {
                self.changed_dict(at.position(), dict, old, new, children)
            }
            (Item::Array(array), Value::Array(old), Value::Array(new)) => {
                self.begin_array()?;
                for ((index, value), plan) in new.iter().enumerate().zip(children) {
                    match (array.get(index), old.get(index)) {
                        (Some(at), Some(old)) => self.changed(at, old, value, plan)?,
                        _ => self.write_value(value)?,
                    }
                }
                self.end_array()
            }
            _ => self.write_value(new),
        }
    }

    fn changed_dict(
        &mut self,
        position: usize,
        dict: DictRef<'_>,
        old: &Object,
        new: &Object,
        children: &[Plan],
    ) -> Result<(), EncodeError> {
        let deleted = || old.keys().filter(|key| !new.contains_key(*key));
        let rewritten = children.iter().filter(|plan| !matches!(plan, Plan::Same)).count();
        // A dictionary of which less than half changes is written as its changes over the
        // old one, which costs an entry more and saves all that stay.
        let inherit = 2 * (rewritten + deleted().count() + 1) < new.len()
            && dict.inherit_depth() < MAX_INHERIT_DEPTH;
        if inherit {
            self.begin_dict_over(position)?;
        } else {
            self.begin_dict()?;
        }
        for ((key, value), plan) in new.iter().zip(children) {
            if inherit && matches!(plan, Plan::Same) {
                continue;
            }
            // A key the old dictionary has is pointed at, not written again.
            self.write_key_at(key, dict.key_position(key))?;
            match (dict.get(key), old.get(key)) {
                (Some(at), Some(old)) => self.changed(at, old, value, plan)?,
                _ => self.write_value(value)?,
            }
        }
        if inherit {
            for key in deleted() {
                self.write_key_at(key, dict.key_position(key))?;
                self.write_undefined()?;
            }
        }
        self.end_dict()
    }
}

/// A value that replaces another one whole. An array or an object is wrapped in an array of
/// one item, to tell it from a delta of items and from a deletion.
fn replacement(value: &Value) -> Value {
    match value {
        Value::Array(_) | Value::Object(_) => Value::Array(vec![value.clone()]),
        _ => value.clone(),
    }
}

/// The delta that turns `old` into `new`, or `None` when they are equal.
///
/// A delta is one of:
/// - a value that is not an array or an object: the new value;
/// - `[value]`: the new value, whatever its type;
/// - `[]`: the field is deleted;
/// - an object: deltas for the fields of an object, by name, or for the items of an array,
///   by index (`"3"`), with `"n-"` holding the items that replace all from index `n` on.
///
/// Arrays are compared item by item at the same index, so an insertion at the front of an
/// array changes every item after it.
pub fn diff(old: &Value, new: &Value) -> Option<Value> {
    let mut delta = Object::new();
    match (old, new) {
        (Value::Object(old), Value::Object(new)) => {
            for (key, value) in new {
                let change = match old.get(key) {
                    Some(old) => diff(old, value),
                    None => Some(replacement(value)),
                };
                delta.extend(change.map(|change| (key.clone(), change)));
            }
            let deleted = old.keys().filter(|key| !new.contains_key(*key));
            delta.extend(deleted.map(|key| (key.clone(), Value::Array(Vec::new()))));
        }
        (Value::Array(old), Value::Array(new)) => {
            let common = old.len().min(new.len());
            for (index, (old, new)) in old.iter().zip(new).enumerate() {
                delta.extend(diff(old, new).map(|change| (index.to_string(), change)));
            }
            if old.len() != new.len() {
                delta.insert(format!("{common}-"), Value::Array(new[common..].to_vec()));
            }
        }
        _ => return (old != new).then(|| replacement(new)),
    }
    (!delta.is_empty()).then_some(Value::Object(delta))
}

/// The value `delta` turns `old` into. `delta` is what [`diff`] gave for `old`, or a delta
/// of Fleece's own: the byte patches of strings those may hold (`["4=1-1+d|30=", 0, 2]`) are
/// applied as well.
pub fn apply(old: &Value, delta: &Value) -> Result<Value, DeltaError> {
    apply_to(Some(old), delta)?.ok_or(DeltaError::Mismatch)
}

/// The new value of a field or item, or `None` when the delta deletes it.
fn apply_to(old: Option<&Value>, delta: &Value) -> Result<Option<Value>, DeltaError> {
    let zero_two = [Value::Number(Number::Int(0)), Value::Number(Number::Int(2))];
    Ok(Some(match delta {
        Value::Array(items) => match &items[..] {
            [] => return Ok(None),
            [value] => value.clone(),
            [Value::String(patch), flags @ ..] if *flags == zero_two => match old {
                Some(Value::String(old)) => Value::String(patch_string(old, patch)?),
                _ => return Err(DeltaError::Mismatch),
            },
            _ => return Err(DeltaError::Malformed),
        },
        Value::Object(changes) => match old {
            Some(Value::Object(old)) => {
                let mut new = old.clone();
                for (key, change) in changes {
                    match apply_to(old.get(key), change)? {
                        Some(value) => new.insert(key.clone(), value),
                        None => new.remove(key),
                    };
                }
                Value::Object(new)
            }
            Some(Value::Array(old)) => Value::Array(apply_to_array(old, changes)?),
            _ => return Err(DeltaError::Mismatch),
        },
        value => value.clone(),
    }))
}

fn apply_to_array(old: &[Value], changes: &Object) -> Result<Vec<Value>, DeltaError> {
    let mut new = old.to_vec();
    let index = |text: &str| text.parse::<usize>().map_err(|_| DeltaError::Malformed);
    let mut tail = None;
    for (key, change) in changes {
        if let Some(from) = key.strip_suffix('-') {
            let Value::Array(items) = change else {
                return Err(DeltaError::Malformed);
            };
            tail = Some((index(from)?, items));
            continue;
        }
        let index = index(key)?;
        let item = old.get(index).ok_or(DeltaError::Mismatch)?;
        new[index] = apply_to(Some(item), change)?.ok_or(DeltaError::Malformed)?;
    }
    if let Some((from, items)) = tail {
        if from > new.len() {
            return Err(DeltaError::Mismatch);
        }
        new.truncate(from);
        new.extend_from_slice(items);
    }
    Ok(new)
}

/// Applies a patch of a string: a series of `n=` (keep n bytes), `n-` (skip n bytes) and
/// `n+text|` (insert the n bytes of text), which together cover the old string.
fn patch_string(old: &str, patch: &str) -> Result<String, DeltaError> {
    let (old, mut patch) = (old.as_bytes(), patch.as_bytes());
    let mut new = Vec::new();
    let mut at = 0usize;
    while !patch.is_empty() {
        let digits = patch.iter().take_while(|b| b.is_ascii_digit()).count();
        let count = std::str::from_utf8(&patch[..digits]).ok().and_then(|n| n.parse().ok());
        let count: usize = count.ok_or(DeltaError::Malformed)?;
        let operation = *patch.get(digits).ok_or(DeltaError::Malformed)?;
        patch = &patch[digits + 1..];
        let end = at.checked_add(count).ok_or(DeltaError::Malformed)?;
        match operation {
            b'=' => {
                new.extend_from_slice(old.get(at..end).ok_or(DeltaError::Mismatch)?);
                at = end;
            }
            b'-' => at = end,
            b'+' => {
                new.extend_from_slice(patch.get(..count).ok_or(DeltaError::Malformed)?);
                if patch.get(count) != Some(&b'|') {
                    return Err(DeltaError::Malformed);
                }
                patch = &patch[count + 1..];
            }
            _ => return Err(DeltaError::Malformed),
        }
    }
    if at != old.len() {
        return Err(DeltaError::Mismatch);
    }
    String::from_utf8(new).map_err(|_| DeltaError::Mismatch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_patches_of_fleece_apply() {
        let old = Value::from("The fog comes in on little cat feet");
        let patch = Value::Array(vec!["4=1-1+d|30=".into(), 0.into(), 2.into()]);
        assert_eq!(apply(&old, &patch), Ok("The dog comes in on little cat feet".into()));
        let short = Value::Array(vec!["4=".into(), 0.into(), 2.into()]);
        assert_eq!(apply(&old, &short), Err(DeltaError::Mismatch));
    }
}
