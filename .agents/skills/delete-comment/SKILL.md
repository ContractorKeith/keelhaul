---
name: delete-comment
description: "Sweep keelhaul's code comments: categorise each as tautology, stale, excuse or decision record, delete it, file an issue per excuse, move decision records into docs/adr/, and lower the comment ratchet. Use for '/delete-comment', 'delete comments', 'run the comment sweep', or when tests/no_comments.rs says the baseline can drop."
---

# delete-comment — keelhaul

keelhaul bans code comments. `tests/no_comments.rs` counts comment lines per file under
`src/`, `tests/`, `scripts/` and `.github/` and compares them with
`tests/comment_baseline.txt`. A file may never gain comment lines, and a file that loses
some must have its baseline lowered in the same PR. This skill drives the baseline toward
zero.

## Scope

Argument: paths or globs (default: every file listed in `tests/comment_baseline.txt`).
Start from the baseline; it is the inventory. Largest counts first.

What counts: Rust `//`, `/* */`, `///`, `//!` (string-aware); in shell, Python and YAML, any
line whose first non-blank character is `#`, except a line-1 shebang. The `#` counter is
line-based: it misses trailing `# ...` and counts `#` lines inside heredocs, docstrings and
YAML block scalars. Only git-tracked files are counted. Not comments:
`#[allow(..., reason = "...")]`, Rust string contents.

## Categorise every block

A contiguous run of comment lines is one block. Read the code around it before deciding.

| Category | Test | Action |
|---|---|---|
| Tautology | Repeats what the name, type or next line already says. | Delete. |
| Stale | Names a milestone (M1…M8), old PR state, or behaviour the code no longer has. | Delete. |
| Excuse | Justifies a hack, workaround, shortcut or known limitation a proper fix would remove. | Delete, file an issue. |
| Decision record | An important, non-obvious why: a format or safety invariant, a portability or threat-model choice, a trade-off a maintainer would undo by mistake. | Delete, move into an ADR. |

Keep a comment, and leave it in the baseline, only when deleting it changes behaviour or
loses a test:

- `///` on clap `derive(Parser, Args, Subcommand)` items is `--help` text. Moving it into
  `#[arg(help = "...")]` / `#[command(about = "...")]` is the fix; do that only when the
  help output stays byte-identical (`tests/skeptic_help.rs` and friends pin it).
- ```` ```compile_fail ```` doc tests (e.g. `src/backend/s3/config.rs`) are tests. Move them
  to a `trybuild`-free form only with a reason in the PR; otherwise keep.

When unsure, keep it and say why in the PR body. A wrong deletion of a decision is worse
than a comment that survives one more sweep.

## Excuses → issues

One issue per excuse, labelled `comment-debt` (plus `bug` if the behaviour is wrong). Body: file:line at the base SHA, the comment verbatim, the
code it excused, and the proper fix. Title: what to fix, not "remove comment". Check
`gh issue list --search` first so a second sweep does not file duplicates.

## Decision records → ADRs

- Read `docs/adr/` first. Extend an existing ADR when the decision belongs to it.
- Otherwise group related decisions into one new ADR: `docs/adr/NNNN-<slug>.md`, next free
  number, same style as 0001 (a title, then one or two plain paragraphs: the decision, the
  forces, the cost, what was rejected). Name the code it governs by module, not line.
- A decision that is really part of the format belongs in `docs/SPEC.md`, which changes
  first in its own PR (AGENTS.md). Note it in the PR and leave the comment until then.
- Update the ADR list in `AGENTS.md` Pointers.

## Lower the ratchet

```bash
export TMPDIR=$PWD/.tmp; mkdir -p .tmp
KEELHAUL_UPDATE_COMMENT_BASELINE=1 cargo test --locked --test no_comments
git diff tests/comment_baseline.txt            # every line must go down or disappear
scripts/accept.sh
```

The update refuses to raise the total, so it cannot launder a new comment; a rename moves
a file's count to its new path. The diff of a sweep is deletions only, plus new ADR files
and the baseline. `git diff -U0 -- src tests ':!tests/comment_baseline.txt' | grep
'^+[^+]'` should print nothing except trailing-comment lines with the comment stripped and
clap help moved into attributes.

## Parallel sweeps

Split by directory (`src/backend/`, rest of `src/`, `tests/` + scripts) so workers touch
disjoint files. Workers delete and write a report (table of every block with category and
reason, plus verbatim text for excuses, decisions and kept comments); the orchestrator alone
numbers ADRs, files issues and regenerates the baseline, so nothing collides.

## Report

PR body: counts per category, lines shed, issues filed (links), ADRs added, comments kept
and why, and the remaining baseline total.
