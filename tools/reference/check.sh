#!/bin/sh
# Checks the crate against the reference implementation, couchbase/fleece: documents each
# reads from the other, deltas each applies from the other, and appended deltas the
# reference reads. Builds the reference first when it is not there (see build.sh).
# Fails when either check does.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
here="$root/tools/reference"

if [ ! -x "$root/target/reference/fleece/build/fleeceTool" ] || [ ! -x "$root/target/reference/delta" ]; then
    "$here/build.sh"
fi
cargo build --quiet --release --example fleece --manifest-path "$root/Cargo.toml"

python3 "$here/documents.py"
python3 "$here/deltas.py"
