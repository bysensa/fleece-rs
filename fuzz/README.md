# Fuzzing

The property tests of `tests/` make their inputs blindly. These targets are run by
[cargo-fuzz](https://github.com/rust-fuzz/cargo-fuzz) (libFuzzer), which keeps the inputs
that reach new code and changes those further, and so gets to branches random data does
not.

| Target | Input | What has to hold |
|---|---|---|
| `read` | any bytes | Unchecked, they are read in place and decoded without a panic and without more values than the bytes have room for. When `Doc::new` accepts them, the value is decoded, and written, compacted and passed through serde it is the same value. |
| `delta` | two documents | `apply(old, diff(old, new))` is `new`; a delta appended to the first document reads as the second, compacted too. |
| `update` | a document, a path, a value | The updated document passes the check and holds the value at the path; a part taken out is what the path reads. |
| `json` | any text | The value JSON gives is written and read back the same, and so is the JSON of that value. |
| `shared` | a table of shared keys, a document | The table stored again is the same; a document checked with it is decoded, and written with it reads the same. |

## Running

Needs a nightly toolchain and `cargo install cargo-fuzz`.

```sh
cargo run --example fuzz_seeds                 # the inputs to start from, once
cd fuzz
cargo +nightly fuzz run -s none -a read -- -max_total_time=120 -max_len=4096
```

`-s none` leaves the sanitizers out: the crate forbids `unsafe`, so there is no memory for
them to watch, and the run is several times faster. `-a` turns the debug assertions on, and
with them the checks for arithmetic that overflows, which a release build does not make.

A finding is written to `fuzz/artifacts/<target>/`. Fix it, and pin it with a test in
`tests/` that holds the bytes: the corpus and the artifacts are not kept in the repository.
