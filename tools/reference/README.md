# The check against couchbase/fleece

The bytes of standard Fleece are checked here against the implementation they come from.

```sh
tools/reference/check.sh
```

It builds the reference at a fixed commit into `target/reference` when it is not there
(`build.sh`; needs git, cmake, a C++ compiler and python3), builds the crate's own tool
(`examples/fleece.rs`), and runs two checks, each over the same documents on every run:

- `documents.py`: 307 documents written by each side and read by the other. The count of
  documents both write as the same bytes is printed; it is 85, and the rest, laid out
  differently, are read the same by both.
- `deltas.py`: 400 pairs of documents, the delta of each side applied by the other; then
  200 documents with 6 deltas appended to each by the crate, dictionaries that inherit
  among them, read by the reference after every one and once more after `compact`.

It exits with 1 when any of them fails. `delta.cc` is a helper over the reference's delta
functions, which its tool does not give.

The extension for the types of SurrealDB has no reference to check against: those bytes
are frozen in `tests/golden.rs`.
