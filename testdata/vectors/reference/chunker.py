#!/usr/bin/env python3
"""Independent SPEC §6 transcription; optional `blake3` package supplies only hashing.
Run with python3 after installing blake3 in an isolated environment. Exit 77 means
that independent BLAKE3 is unavailable; the Rust test prints a skip note then.
"""
import json
from pathlib import Path
import struct
import sys
try:
    from blake3 import blake3
except ImportError:
    print("reference chunker: skipped (Python blake3 unavailable)")
    sys.exit(77)

root = Path(__file__).resolve().parent.parent
keys = json.loads((root / "subkeys.json").read_text())
raw = blake3(key=bytes.fromhex(keys["gear"])).digest(length=2048)
gear = struct.unpack("<256Q", raw)
expected = json.loads((root / "gear.json").read_text())
assert list(gear[:8]) == expected["first8"]
assert blake3(raw).hexdigest() == expected["blake3"]
v = json.loads((root / "chunks.json").read_text())
stream = blake3(v["seed"].encode()).digest(length=v["len"])
lengths = []
s = 0
while s < len(stream):
    n = len(stream) - s
    end = min(n, 8388608)
    normal = min(end, 1048576)
    h = 0
    length = end
    if n > 524288:
        for i in range(524288, end):
            h = ((h << 1) + gear[stream[s+i]]) & 0xffffffffffffffff
            mask = 0xfffff80000000000 if i < normal else 0xffffe00000000000
            if h & mask == 0:
                length = i+1
                break
    lengths.append(length)
    s += length
assert lengths == v["lengths"], (lengths, v["lengths"])
print("reference chunker: gear and 32 MiB cut points match")
