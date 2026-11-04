#!/usr/bin/env python3
"""Deliberately break five independent checks, require harness failures, restore."""
import hashlib
import os
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parent.parent
os.chdir(ROOT)
LOGS = ROOT / '.tmp' / 'damage-mutations'
LOGS.mkdir(parents=True, exist_ok=True)
ENV = dict(os.environ, KEELHAUL_SEED='0', TMPDIR=str(ROOT / '.tmp'))
CASES = [
    ('drop-keyed-blob-id', 'src/repo/audit.rs',
     'if blake3::keyed_hash(&keys.chunk_id, &payload)',
     'if false && blake3::keyed_hash(&keys.chunk_id, &payload)'),
    ('ignore-index-header-mismatch', 'src/ops/check/inventory.rs',
     'Some(Some(header)) if header.blobs != pack.blobs',
     'Some(Some(header)) if false && header.blobs != pack.blobs'),
    ('tree-read-failure-as-empty', 'src/ops/check/mod.rs',
     'let tree = self.report.collect(result)?.map(Rc::new);',
     'let result=result.or_else(|e| match e {Error::Damage{..}=>Ok(Tree{version:1,entries:vec![]}),other=>Err(other)});\n'
     'let tree = self.report.collect(result)?.map(Rc::new);'),
    ('drop-pack-hash', 'src/ops/verify.rs',
     'if blake3::hash(&bytes).to_hex().as_str() != id',
     'if false && blake3::hash(&bytes).to_hex().as_str() != id'),
    ('drop-missing-pack', 'src/ops/check/inventory.rs',
     'None => {\n                    state.report.add(\n'
     '                        crate::repo::blobs::pack_name(&pack.id)?.as_str(),\n'
     '                        "pack",\n                        "missing",\n'
     '                        "indexed pack is missing",\n                    );\n                }',
     'None => {}'),
]
COMMAND = ['cargo', 'test', '--locked', '--release', '--test', 'damage', '--', '--nocapture']
original = {path: (ROOT / path).read_bytes() for _, path, _, _ in CASES}
results = []
try:
    for name, path, old, new in CASES:
        source = original[path].decode()
        assert source.count(old) == 1, f'{name}: mutation seam changed'
        (ROOT / path).write_text(source.replace(old, new, 1))
        start = time.monotonic()
        try:
            run = subprocess.run(COMMAND, env=ENV, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        finally:
            (ROOT / path).write_bytes(original[path])
        output = run.stdout.decode(errors='replace')
        (LOGS / f'{name}.log').write_text(output)
        killed = (run.returncode == 101 and 'seeded_damage_exact_sets ... FAILED' in output
                  and 'assertion' in output and 'could not compile' not in output)
        seconds = time.monotonic() - start
        results.append((name, killed, run.returncode, seconds))
        print(f'{name}: {"KILLED" if killed else "SURVIVED/INVALID"}, exit {run.returncode}, {seconds:.2f}s', flush=True)
finally:
    for path, content in original.items():
        (ROOT / path).write_bytes(content)
    assert all(hashlib.sha256((ROOT / p).read_bytes()).digest() == hashlib.sha256(b).digest()
               for p, b in original.items())
    print('restored original sources: yes', flush=True)
assert all(killed for _, killed, _, _ in results), 'mutation gate failed; inspect .tmp/damage-mutations/'
run = subprocess.run(COMMAND, env=ENV, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
(LOGS / 'restored-baseline.log').write_bytes(run.stdout)
assert run.returncode == 0, run.stdout.decode(errors='replace')
print('restored baseline: PASS', flush=True)
if ENV.get('KEELHAUL_MUTATION_REPORT') == '0':
    raise SystemExit(0)
rows = '\n'.join(f'| {name} | KILLED (exit {code}) | {seconds:.2f}s |' for name, _, code, seconds in results)
(ROOT / 'tests/damage_mutations.md').write_text('''# Damage harness mutation gate\n\nRun `python3 tests/run_damage_mutations.py`. Each mutant runs\n`KEELHAUL_SEED=0 cargo test --locked --release --test damage -- --nocapture`.\nFailures must be harness assertion failures, not compiler errors. The runner restores\neach source after its run, verifies the original source hashes, then requires a green\nunmodified baseline. Full output is saved under `.tmp/damage-mutations/`.\n\n| Mutation | Observed result | Wall time |\n|---|---|---|\n''' + rows + '''\n\nThe mutations remove the keyed blob-ID comparison, suppress index/header disagreement,\nturn a failed tree read into an empty tree, remove whole-pack hash validation, and\nsuppress the missing-pack report. Exact `(object, kind, problem)` sets reject all five.\nThe re-sealed descriptor permutation makes index disagreement observable; the\nequal-length authenticated envelope swap makes keyed-ID validation independently\nobservable. Pack hashes and missing packs have their own expected signatures.\n\nOriginal sources restored: yes. Unmodified baseline: PASS.\n''')
