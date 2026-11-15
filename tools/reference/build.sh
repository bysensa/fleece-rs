#!/bin/sh
# Builds the reference implementation, couchbase/fleece, into target/reference: its tool
# `fleeceTool`, and `delta`, the helper of delta.cc over its delta functions.
# Needs git, cmake and a C++ compiler.
set -eu

root=$(cd "$(dirname "$0")/../.." && pwd)
dir="$root/target/reference"
# The commit the crate was checked against. Another one is another check: say so in the
# commit that changes it.
commit=51283bf57ae774aa2eda65f05c8ba30eee349350

if [ ! -d "$dir/fleece/.git" ]; then
    mkdir -p "$dir"
    git clone --quiet https://github.com/couchbase/fleece.git "$dir/fleece"
fi
git -C "$dir/fleece" checkout --quiet "$commit"
# Without its submodules the reference does not build.
git -C "$dir/fleece" submodule update --init --quiet

cmake -S "$dir/fleece" -B "$dir/fleece/build" -DCMAKE_BUILD_TYPE=Release > "$dir/cmake.log" 2>&1
cmake --build "$dir/fleece/build" -j 8 > "$dir/build.log" 2>&1 || {
    tail -20 "$dir/build.log" >&2
    exit 1
}

frameworks=""
if [ "$(uname)" = Darwin ]; then
    frameworks="-framework CoreFoundation -framework Foundation"
fi
# shellcheck disable=SC2086 # the frameworks are separate arguments
c++ -std=c++20 -O2 -I"$dir/fleece/API" "$root/tools/reference/delta.cc" \
    "$dir/fleece/build/libFleeceStatic.a" $frameworks -o "$dir/delta"

echo "the reference at $commit is built in $dir"
