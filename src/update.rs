//! Changing a document without decoding it.
//!
//! A store that keeps a document as one value writes the whole value on every change, so
//! what a change costs is making the new bytes. [`update`] makes them from the old ones in
//! place: it copies the document and writes the changed values on the way, without building
//! a [`Value`] of what stays. [`extract`] gives one part of a document as a document.

use std::collections::BTreeMap;

use crate::encoder::Encoder;
use crate::error::DeltaError;
use crate::keypath::{KeyPath, Step};
use crate::reader::{Doc, Item, ValueRef};
use crate::value::Value;

/// The changes of one document: values set and removed, by path.
///
/// ```
/// # fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use fleece::{Doc, Patch, Value};
///
/// let object = |fields: Vec<(&str, Value)>| {
///     Value::Object(fields.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())
/// };
/// // {"name": "Milou", "tags": ["dog"], "age": 9}
/// let tags = Value::Array(vec!["dog".into()]);
/// let old = object(vec![("name", "Milou".into()), ("tags", tags), ("age", 9.into())]);
/// let old = fleece::encode(&old)?;
///
/// let (ten, white) = (Value::from(10), Value::from("white"));
/// let mut patch = Patch::new();
/// patch.set(&"age".parse()?, &ten);
/// patch.set(&"coat.colour".parse()?, &white);
/// patch.remove(&"tags[0]".parse()?);
/// let new = fleece::update(&Doc::new(&old)?, &patch)?;
///
/// // {"name": "Milou", "tags": [], "age": 10, "coat": {"colour": "white"}}
/// let coat = object(vec![("colour", "white".into())]);
/// let expected = object(vec![
///     ("name", "Milou".into()),
///     ("tags", Value::Array(vec![])),
///     ("age", 10.into()),
///     ("coat", coat),
/// ]);
/// assert_eq!(fleece::decode(&new)?, expected);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Debug, Default)]
pub struct Patch<'v> {
    root: Node<'v>,
}

#[derive(Clone, Debug)]
enum Node<'v> {
    Set(&'v Value),
    Remove,
    /// Changes further down: of the fields of an object, or of the items of an array.
    Inside {
        fields: BTreeMap<String, Node<'v>>,
        items: BTreeMap<i64, Node<'v>>,
    },
}

impl Default for Node<'_> {
    fn default() -> Self {
        Self::Inside { fields: BTreeMap::new(), items: BTreeMap::new() }
    }
}

impl<'v> Patch<'v> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the patch changes nothing.
    pub fn is_empty(&self) -> bool {
        matches!(&self.root, Node::Inside { fields, items } if fields.is_empty() && items.is_empty())
    }

    /// Sets the value at `path`. An object on the way that is not there is made; an index
    /// equal to the length of an array adds an item to it.
    pub fn set(&mut self, path: &KeyPath, value: &'v Value) -> &mut Self {
        self.put(path, Node::Set(value))
    }

    /// Removes the field or the item at `path`. Removing what is not there changes nothing.
    pub fn remove(&mut self, path: &KeyPath) -> &mut Self {
        self.put(path, Node::Remove)
    }

    /// Of two changes of one path the later stays, and a change below a path replaces a
    /// change of that path made before.
    fn put(&mut self, path: &KeyPath, change: Node<'v>) -> &mut Self {
        let mut node = &mut self.root;
        for step in path.steps() {
            if !matches!(node, Node::Inside { .. }) {
                *node = Node::default();
            }
            let Node::Inside { fields, items } = node else {
                return self;
            };
            node = match step {
                Step::Key(key) => fields.entry(key.clone()).or_default(),
                Step::Index(index) => items.entry(*index).or_default(),
            };
        }
        *node = change;
        self
    }
}

/// The document with the changes of `patch`, written anew and compact. The document is not
/// checked here: give one that [`Doc::new`] checked or whose bytes are trusted.
///
/// A path that does not fit the document (a field of what is not an object, an index past
/// the end of an array) is refused with [`DeltaError::Mismatch`].
pub fn update(doc: &Doc<'_>, patch: &Patch<'_>) -> Result<Vec<u8>, DeltaError> {
    let mut encoder = encoder_for(doc);
    encoder.write_updated(doc.root(), patch)?;
    Ok(encoder.finish()?)
}

/// The value at `path` as a document of its own, or `None` when the path leads nowhere.
pub fn extract(doc: &Doc<'_>, path: &KeyPath) -> Result<Option<Vec<u8>>, DeltaError> {
    let Some(value) = path.eval(doc.root()) else {
        return Ok(None);
    };
    let mut encoder = encoder_for(doc);
    encoder.write_copy(value)?;
    Ok(Some(encoder.finish()?))
}

/// An encoder that keeps the keys of `doc` the numbers they are.
fn encoder_for<'k>(doc: &Doc<'k>) -> Encoder<'k> {
    match doc.shared() {
        Some(shared) => Encoder::new().with_fixed_shared_keys(shared),
        None => Encoder::new(),
    }
}

impl Encoder<'_> {
    /// Writes a copy of `value` with the changes of `patch`.
    pub fn write_updated(
        &mut self,
        value: ValueRef<'_>,
        patch: &Patch<'_>,
    ) -> Result<(), DeltaError> {
        self.patched(Some(value), &patch.root)
    }

    /// Writes what `node` makes of the value at a place, which may hold none yet.
    fn patched(&mut self, at: Option<ValueRef<'_>>, node: &Node<'_>) -> Result<(), DeltaError> {
        let (fields, items) = match node {
            Node::Set(value) => return Ok(self.write_value(value)?),
            Node::Remove => return Err(DeltaError::Mismatch),
            Node::Inside { fields, items } => (fields, items),
        };
        match at.map(|value| (value, value.item())) {
            Some((value, _)) if fields.is_empty() && items.is_empty() => {
                Ok(self.write_copy(value)?)
            }
            Some((_, Item::Dict(dict))) if items.is_empty() => {
                self.begin_dict()?;
                let mut entries = 0;
                for (key, value) in dict.iter() {
                    entries += 1;
                    match fields.get(key) {
                        Some(Node::Remove) => continue,
                        Some(change) => {
                            self.write_key(key)?;
                            self.patched(Some(value), change)?;
                        }
                        None => {
                            self.write_key(key)?;
                            self.write_copy(value)?;
                        }
                    }
                }
                if !dict.gave_all(entries) {
                    return Err(DeltaError::Mismatch);
                }
                let added = fields.iter().filter(|(key, _)| dict.get(key).is_none());
                self.added_fields(added)?;
                Ok(self.end_dict()?)
            }
            Some((_, Item::Array(array))) if fields.is_empty() => {
                // An index below zero counts from the end, and one equal to the length
                // is the place after the last item.
                let mut changes = BTreeMap::new();
                for (index, change) in items {
                    let index = match usize::try_from(*index) {
                        Ok(index) => Some(index),
                        Err(_) => array.len().checked_sub(index.unsigned_abs() as usize),
                    };
                    let index = index.filter(|index| *index <= array.len());
                    changes.insert(index.ok_or(DeltaError::Mismatch)?, change);
                }
                self.begin_array()?;
                for (index, item) in array.iter().enumerate() {
                    match changes.get(&index) {
                        Some(Node::Remove) => {}
                        Some(change) => self.patched(Some(item), change)?,
                        None => self.write_copy(item)?,
                    }
                }
                match changes.get(&array.len()) {
                    None | Some(Node::Remove) => {}
                    Some(change) => self.patched(None, change)?,
                }
                Ok(self.end_array()?)
            }
            // Nothing is there: the fields set below make a new object.
            None if items.is_empty() => {
                self.begin_dict()?;
                self.added_fields(fields.iter())?;
                Ok(self.end_dict()?)
            }
            _ => Err(DeltaError::Mismatch),
        }
    }

    fn added_fields<'n, 'v: 'n>(
        &mut self,
        fields: impl Iterator<Item = (&'n String, &'n Node<'v>)>,
    ) -> Result<(), DeltaError> {
        for (key, change) in fields {
            if !matches!(change, Node::Remove) {
                self.write_key(key)?;
                self.patched(None, change)?;
            }
        }
        Ok(())
    }
}
