#!/usr/bin/env python3
"""Prove deletion ordering and supersedes assertions fail, then restore exact sources."""
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parent.parent
LOGS = ROOT / '.tmp' / 'prune-mutations'
LOGS.mkdir(parents=True, exist_ok=True)
ENV = dict(os.environ, TMPDIR=str(ROOT / '.tmp'))
CASES = [
    ('deletes-before-replacement-index', 'src/ops/prune.rs',
     '    // Publish replacements before deleting any old object.',
     '    for entry in &old_indexes { repo.backend.delete(&entry.name)?; }\n'
     '    // Publish replacements before deleting any old object.',
     ['--test', 'crash', 'panic_after_first_delete_keeps_snapshot_restorable']),
    ('missing-supersedes', 'src/ops/prune/indexes.rs',
     '    index.supersedes = supersedes;',
     '    index.supersedes.clear();',
     ['--test', 'm5_prune', 'replacement_supersedes_scanned_and_repack_flushed_indexes_exactly']),
]
original = {path: (ROOT / path).read_bytes() for _, path, *_ in CASES}
try:
    for name, path, old, new, target in CASES:
        source = original[path].decode()
        assert source.count(old) == 1, f'{name}: mutation seam changed'
        (ROOT / path).write_text(source.replace(old, new, 1))
        try:
            run = subprocess.run(['cargo', 'test', '--locked', '--release', *target,
                                  '--', '--nocapture'], cwd=ROOT, env=ENV,
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        finally:
            (ROOT / path).write_bytes(original[path])
        output = run.stdout.decode(errors='replace')
        (LOGS / f'{name}.log').write_text(output)
        caught = (run.returncode == 101 and 'test result: FAILED' in output
                  and 'could not compile' not in output)
        print(f'{name}: {"caught" if caught else "SURVIVED/INVALID"}', flush=True)
        assert caught, f'{name}: mutation was not caught; see {LOGS}'
finally:
    for path, content in original.items():
        (ROOT / path).write_bytes(content)
