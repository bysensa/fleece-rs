"""Documents of the crate against the `fleeceTool` of couchbase/fleece: each reads what the
other wrote. Run by check.sh."""
import json
import random
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CPP = str(ROOT / "target/reference/fleece/build/fleeceTool")
RS = str(ROOT / "target/release/examples/fleece")

# The same documents on every run.
random.seed(20261115)


def key():
    return random.choice(["a", "", "name", "héllo", "k%d" % random.randint(0, 50), "x" * random.randint(1, 30)])


def rnd(depth=0):
    r = random.random()
    if depth < 4 and r < 0.2:
        return [rnd(depth + 1) for _ in range(random.randint(0, 6))]
    if depth < 4 and r < 0.4:
        return {key(): rnd(depth + 1) for _ in range(random.randint(0, 6))}
    return random.choice([
        None, True, False,
        random.randint(-3000, 3000), random.randint(-2**62, 2**62), random.randint(0, 2**63 - 1),
        random.random() * 1e6, 1.5, -0.25, 1e300, 0.1,
        "", "a", "héllo wörld ✓", "x" * random.randint(0, 40),
    ])


docs = {
    "example": {"hello": "world!", "time": 1234567890, "float": 0.01234, "boolean": True,
                "otherbool": False, "null": None, "obj": {"what": "that"}, "arr": [1, 2, 3]},
    "long_array": list(range(5000)),
    "wide": ["string number %06d padded to be long enough" % i for i in range(3000)],
    "far_root": [i % 100 for i in range(40000)],
    "many_keys": {"key%05d" % i: i for i in range(3000)},
    "scalars": [0, -1, 2047, 2048, -2048, -2049, 2**31, -2**31, 2**63 - 1, -2**63,
                0.5, 1e-10, 3.141592653589793, -0.0, 1.5],
    "nested": {"a": {"b": {"c": {"d": [[[[1]]]]}}}},
}
for i in range(300):
    # The reference tool takes only an array or an object at the top.
    docs["random%03d" % i] = [rnd() for _ in range(random.randint(1, 8))]


def run(cmd, data):
    done = subprocess.run(cmd, input=data, capture_output=True)
    if done.returncode != 0:
        raise RuntimeError("%s failed: %s" % (cmd, done.stderr.decode()[:300]))
    return done.stdout


failures = 0
same_bytes = 0
for name, doc in docs.items():
    src = json.dumps(doc).encode()
    try:
        cpp_bytes = run([CPP, "encode"], src)
        rs_bytes = run([RS, "encode"], src)
        from_cpp = json.loads(run([RS, "decode"], cpp_bytes))
        from_rs = json.loads(run([CPP, "decode"], rs_bytes))
        same_bytes += cpp_bytes == rs_bytes
        if from_cpp != doc:
            failures += 1
            print("FAIL %s: Rust reads C++ bytes differently" % name)
        if from_rs != doc:
            failures += 1
            print("FAIL %s: C++ reads Rust bytes differently" % name)
    except Exception as e:  # noqa: BLE001
        failures += 1
        print("FAIL %s: %s" % (name, e))

print("%d documents, %d failures, %d with identical bytes" % (len(docs), failures, same_bytes))
sys.exit(1 if failures else 0)
