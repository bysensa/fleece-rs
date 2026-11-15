//! A path into a document: `foo.bar[2][-3].baz`.

use std::fmt::{self, Display, Formatter};
use std::str::FromStr;

use crate::reader::ValueRef;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Key(String),
    Index(i64),
}

/// A series of dictionary keys and array indexes, parsed once and evaluated many times.
///
/// Keys are separated by `.`, indexes stand in `[]`, and a negative index counts from the
/// end of the array. `\` takes the character after it literally, so a key may hold `.`, `[`
/// or `\`. A leading `$.` or `$` before `[`, as JSONPath has it, is left out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPath {
    steps: Vec<Step>,
}

/// The text is not a path: where it stops being one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeyPathError {
    /// The byte offset of the character that was refused.
    pub at: usize,
}

impl Display for KeyPathError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "the path is malformed at byte {}", self.at)
    }
}

impl std::error::Error for KeyPathError {}

impl FromStr for KeyPath {
    type Err = KeyPathError;

    fn from_str(text: &str) -> Result<Self, KeyPathError> {
        let bytes = text.as_bytes();
        let mut at = match bytes {
            [b'$'] | [b'$', b'[', ..] => 1,
            [b'$', b'.', ..] => 2,
            _ => 0,
        };
        let mut steps = Vec::new();
        // Whether a step was just read, so that only `.`, `[` or the end may follow.
        let mut after_step = false;
        while at < bytes.len() {
            let error = KeyPathError { at };
            match bytes[at] {
                b'[' => {
                    let end = at + text[at..].find(']').ok_or(error)?;
                    steps.push(Step::Index(text[at + 1..end].parse().map_err(|_| error)?));
                    at = end + 1;
                    after_step = true;
                }
                b'.' if after_step => {
                    at += 1;
                    if matches!(bytes.get(at), None | Some(b'.' | b'[')) {
                        return Err(KeyPathError { at });
                    }
                    after_step = false;
                }
                _ if after_step => return Err(error),
                b'.' => return Err(error),
                _ => {
                    let mut key = String::new();
                    let mut chars = text[at..].char_indices();
                    let mut len = text.len() - at;
                    while let Some((i, c)) = chars.next() {
                        match c {
                            '\\' => key.push(chars.next().ok_or(KeyPathError { at: at + i })?.1),
                            '.' | '[' => {
                                len = i;
                                break;
                            }
                            _ => key.push(c),
                        }
                    }
                    steps.push(Step::Key(key));
                    at += len;
                    after_step = true;
                }
            }
        }
        Ok(Self { steps })
    }
}

impl KeyPath {
    pub(crate) fn steps(&self) -> &[Step] {
        &self.steps
    }

    /// The value the path leads to from `root`, or `None` when a key is missing, an index is
    /// out of range or a value on the way is not a collection of the kind the step takes.
    pub fn eval<'a>(&self, root: ValueRef<'a>) -> Option<ValueRef<'a>> {
        self.steps.iter().try_fold(root, |value, step| match step {
            Step::Key(key) => value.as_dict()?.get(key),
            Step::Index(index) => {
                let array = value.as_array()?;
                let index = if *index < 0 {
                    array.len().checked_sub(usize::try_from(index.unsigned_abs()).ok()?)?
                } else {
                    usize::try_from(*index).ok()?
                };
                array.get(index)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(text: &str) -> Vec<Step> {
        text.parse::<KeyPath>().unwrap().steps
    }

    fn key(text: &str) -> Step {
        Step::Key(text.to_owned())
    }

    #[test]
    fn parses() {
        assert_eq!(steps(""), []);
        assert_eq!(steps("$"), []);
        assert_eq!(steps("foo"), [key("foo")]);
        assert_eq!(steps("$.foo.bar"), [key("foo"), key("bar")]);
        assert_eq!(
            steps("foo.bar[2][-3].baz"),
            [key("foo"), key("bar"), Step::Index(2), Step::Index(-3), key("baz")]
        );
        assert_eq!(steps("$[0].a"), [Step::Index(0), key("a")]);
        assert_eq!(steps(r"a\.b.\[c\\"), [key("a.b"), key(r"[c\")]);
        assert_eq!(steps("$dollar"), [key("$dollar")]);
    }

    #[test]
    fn refuses() {
        let cases = [(".a", 0), ("a..b", 2), ("a.", 2), ("a[1", 1), ("a[x]", 1), ("a[1]b", 4)];
        for (text, at) in cases {
            assert_eq!(text.parse::<KeyPath>(), Err(KeyPathError { at }), "{text}");
        }
        assert!("a[1].".parse::<KeyPath>().is_err());
        assert!(r"a\".parse::<KeyPath>().is_err());
    }
}
