"""Deltas and inherited dictionaries of the crate against couchbase/fleece: each applies the
delta the other made, and the reference reads the deltas the crate appends. Run by check.sh."""
import json
import os
import random
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CPP_TOOL = str(ROOT / "target/reference/fleece/build/fleeceTool")
CPP_DELTA = str(ROOT / "target/reference/delta")
RS = str(ROOT / "target/release/examples/fleece")

# The same documents on every run.
random.seed(20261115)
tmp = tempfile.mkdtemp(prefix="fleece-deltas-")

WORDS = ["fog", "dog", "comes", "in", "on", "little", "cat", "feet", "city", "wind", "name"]


def text():
    return " ".join(random.choice(WORDS) for _ in range(random.randint(8, 30)))


def key():
    return random.choice(["a", "name", "héllo", "k%d" % random.randint(0, 40), "x" * random.randint(1, 20)])


def rnd(depth=0):
    r = random.random()
    if depth < 3 and r < 0.2:
        return [rnd(depth + 1) for _ in range(random.randint(0, 6))]
    if depth < 3 and r < 0.4:
        return {key(): rnd(depth + 1) for _ in range(random.randint(0, 8))}
    return random.choice([None, True, False, random.randint(-3000, 3000), random.randint(-2**62, 2**62),
                          1.5, -0.25, 0.1, "", "a", "héllo wörld ✓", text(), text()])


def record():
    fields = {"field%02d" % i: rnd(1) for i in range(random.randint(20, 40))}
    fields["text"] = text()
    fields["inner"] = {"field%02d" % i: text() for i in range(30)}
    return fields


def mutate(v, depth=0):
    if isinstance(v, dict):
        v = dict(v)
        for k in list(v):
            r = random.random()
            if r < 0.05:
                del v[k]
            elif r < 0.2:
                v[k] = mutate(v[k], depth + 1)
        if random.random() < 0.3:
            v[key()] = rnd(depth + 1)
        return v
    if isinstance(v, list):
        v = [mutate(x, depth + 1) if random.random() < 0.3 else x for x in v]
        if random.random() < 0.3:
            v = v[:random.randint(0, len(v))]
        if random.random() < 0.3:
            v = v + [rnd(depth + 1)]
        return v
    if isinstance(v, str) and len(v) > 20 and random.random() < 0.8:
        at = random.randint(0, len(v) - 5)
        return v[:at] + random.choice(["", "X", "wörd "]) + v[at + random.randint(0, 4):]
    return rnd(depth + 1)


def run(cmd, data=None):
    done = subprocess.run(cmd, input=data, capture_output=True)
    if done.returncode != 0:
        raise RuntimeError("%s failed (%d): %s" % (cmd[:2], done.returncode, done.stderr.decode()[:200]))
    return done.stdout


def put(name, data):
    path = os.path.join(tmp, name)
    with open(path, "wb") as f:
        f.write(data if isinstance(data, bytes) else json.dumps(data).encode())
    return path


# The example of Deltas.md, as the reference itself makes and takes it. The document gives
# the patch as `31=`, one byte more than the string has left; the reference refuses that
# and takes `30=`, as the crate does.
old = put("fog-old.json", ["The fog comes in on little cat feet"])
new = put("fog-new.json", ["The dog comes in on little cat feet"])
print("reference delta for the fog example:", run([CPP_DELTA, "create", old, new]).decode())
for count in (31, 30):
    patch = put("fog-patch.json", {"0": ["4=1-1+d|%d=" % count, 0, 2]})
    try:
        print("reference applies %d=: %s" % (count, run([CPP_DELTA, "apply", old, patch]).decode()))
    except RuntimeError as e:
        print("reference refuses %d=: %s" % (count, e))

failed = 0
failures = 0
patches = 0
same_delta = 0
total = 400
for i in range(total):
    a = record() if i % 2 else {key(): rnd() for _ in range(random.randint(1, 8))}
    if i % 5 == 0:
        a = [rnd() for _ in range(random.randint(1, 8))]
    b = mutate(a)
    old, new = put("old.json", a), put("new.json", b)
    try:
        cpp_delta = run([CPP_DELTA, "create", old, new])
        patches += b",0,2]" in cpp_delta
        ours = json.loads(run([RS, "apply", old, put("cpp-delta.json", cpp_delta)]))
        if ours != b:
            failures += 1
            print("FAIL %d: we apply the reference's delta differently: %s" % (i, cpp_delta[:200]))
        rs_delta = run([RS, "diff", old, new])
        same_delta += json.loads(rs_delta) == json.loads(cpp_delta)
        theirs = json.loads(run([CPP_DELTA, "apply", old, put("rs-delta.json", rs_delta)]))
        if theirs != b:
            failures += 1
            print("FAIL %d: the reference applies our delta differently: %s" % (i, rs_delta[:200]))
    except Exception as e:  # noqa: BLE001
        failures += 1
        print("FAIL %d: %s" % (i, e))
print("%d pairs, %d failures, %d reference deltas with string patches, %d deltas identical"
      % (total, failures, patches, same_delta))
failed += failures

# Deltas appended here, with dictionaries that inherit, as the reference reads them.
failures = 0
inherited = 0
for i in range(200):
    doc = record()
    base = run([RS, "encode"], json.dumps(doc).encode())
    data = base
    for _ in range(6):
        doc = dict(doc)
        doc["field%02d" % random.randint(0, 19)] = rnd(1)
        if random.random() < 0.5:
            inner = dict(doc["inner"])
            inner["field%02d" % random.randint(0, 29)] = text()
            doc["inner"] = inner
        if random.random() < 0.3:
            doc.pop("field%02d" % random.randint(0, 19), None)
        delta = run([RS, "append", put("base.fleece", data), put("new.json", doc)])
        data += delta
        inherited += b"\x08\x00" in delta
        try:
            if json.loads(run([CPP_TOOL, "decode"], data)) != doc:
                failures += 1
                print("FAIL %d: the reference reads our appended delta differently" % i)
            if json.loads(run([RS, "decode"], data)) != doc:
                failures += 1
                print("FAIL %d: we read our appended delta differently" % i)
        except Exception as e:  # noqa: BLE001
            failures += 1
            print("FAIL %d: %s" % (i, e))
    compacted = run([RS, "compact"], data)
    if json.loads(run([CPP_TOOL, "decode"], compacted)) != doc:
        failures += 1
        print("FAIL %d: the reference reads our compacted document differently" % i)
print("200 documents of 6 appended deltas each, %d failures, %d deltas with an inherited dictionary"
      % (failures, inherited))
failed += failures
sys.exit(1 if failed else 0)
