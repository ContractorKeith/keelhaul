use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

const BASELINE: &str = "tests/comment_baseline.txt";
const UPDATE_ENV: &str = "KEELHAUL_UPDATE_COMMENT_BASELINE";
const SCANNED_DIRS: &[&str] = &["src", "tests", "scripts", ".github"];

#[test]
fn comments_only_go_down() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let counts = scan(root);
    let baseline = parse(&fs::read_to_string(root.join(BASELINE)).expect("read the baseline"));
    let mut problems = Vec::new();
    for (path, &count) in &counts {
        let allowed = baseline.get(path).copied().unwrap_or(0);
        if count > allowed {
            problems.push(format!(
                "{path}: {count} comment lines, baseline allows {allowed}. Delete the comment; \
                 put a decision in docs/adr/, a hack in an issue."
            ));
        }
    }
    let total: usize = counts.values().sum();
    let allowed_total: usize = baseline.values().sum();
    if std::env::var(UPDATE_ENV).is_ok_and(|value| value == "1") {
        assert!(
            total <= allowed_total,
            "comment ratchet: {total} comment lines, baseline allows {allowed_total}; \
             the baseline only goes down\n{}",
            problems.join("\n")
        );
        fs::write(root.join(BASELINE), render(&counts)).expect("write baseline");
        return;
    }
    for (path, &count) in &counts {
        let allowed = baseline.get(path).copied().unwrap_or(0);
        if count < allowed {
            problems.push(format!(
                "{path}: {count} comment lines, baseline says {allowed}. Lower the baseline: \
                 {UPDATE_ENV}=1 cargo test --test no_comments"
            ));
        }
    }
    for (path, &allowed) in &baseline {
        if allowed > 0 && !counts.contains_key(path) {
            problems.push(format!(
                "{path}: no comments left (or file gone), baseline says {allowed}. Lower the \
                 baseline: {UPDATE_ENV}=1 cargo test --test no_comments"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "comment ratchet:\n{}",
        problems.join("\n")
    );
}

fn scan(root: &Path) -> BTreeMap<String, usize> {
    let files = tracked(root).unwrap_or_else(|| {
        let mut files = Vec::new();
        for dir in SCANNED_DIRS {
            walk(&root.join(dir), &mut files);
        }
        files
    });
    let mut counts = BTreeMap::new();
    for file in files {
        let count = match file.extension().and_then(|e| e.to_str()) {
            Some("rs") => rust_comment_lines(&read(&file)),
            Some("sh" | "py" | "yml" | "yaml") => hash_comment_lines(&read(&file)),
            _ => continue,
        };
        if count > 0 {
            let relative = file.strip_prefix(root).expect("under root");
            counts.insert(relative.to_string_lossy().replace('\\', "/"), count);
        }
    }
    counts
}

fn read(file: &Path) -> String {
    let bytes = fs::read(file).unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
    String::from_utf8_lossy(&bytes).into_owned()
}

fn tracked(root: &Path) -> Option<Vec<PathBuf>> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z", "--"])
        .args(SCANNED_DIRS)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let listing = String::from_utf8(output.stdout).ok()?;
    Some(
        listing
            .split('\0')
            .filter(|name| !name.is_empty())
            .map(|name| root.join(name))
            .collect(),
    )
}

fn walk(dir: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, files);
        } else {
            files.push(path);
        }
    }
}

fn parse(text: &str) -> BTreeMap<String, usize> {
    text.lines()
        .filter_map(|line| {
            let (count, path) = line.trim().split_once(' ')?;
            Some((path.trim().to_string(), count.parse().ok()?))
        })
        .collect()
}

fn render(counts: &BTreeMap<String, usize>) -> String {
    counts
        .iter()
        .map(|(path, count)| format!("{count} {path}\n"))
        .collect()
}

fn hash_comment_lines(text: &str) -> usize {
    text.lines()
        .enumerate()
        .filter(|(n, line)| {
            let line = line.trim_start();
            line.starts_with('#') && !(*n == 0 && line.starts_with("#!"))
        })
        .count()
}

fn rust_comment_lines(text: &str) -> usize {
    let chars: Vec<char> = text.chars().collect();
    let mut lines = std::collections::BTreeSet::new();
    let mut line = 0;
    let mut i = 0;
    let at = |i: usize| chars.get(i).copied().unwrap_or('\0');
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c == '/' && at(i + 1) == '/' {
            lines.insert(line);
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c == '/' && at(i + 1) == '*' {
            let mut depth = 0;
            loop {
                if i >= chars.len() {
                    break;
                }
                lines.insert(line);
                if chars[i] == '/' && at(i + 1) == '*' {
                    depth += 1;
                    i += 2;
                } else if chars[i] == '*' && at(i + 1) == '/' {
                    depth -= 1;
                    i += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
        } else if c == 'r' && raw_prefix_ok(&chars, i) && matches!(at(i + 1), '"' | '#') {
            let mut j = i + 1;
            let mut hashes = 0;
            while at(j) == '#' {
                hashes += 1;
                j += 1;
            }
            if at(j) != '"' {
                i += 1;
                continue;
            }
            j += 1;
            while j < chars.len() {
                if chars[j] == '\n' {
                    line += 1;
                }
                if chars[j] == '"' && (1..=hashes).all(|k| at(j + k) == '#') {
                    j += 1 + hashes;
                    break;
                }
                j += 1;
            }
            i = j;
        } else if c == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' {
                    i += 1;
                }
                if at(i) == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i += 1;
        } else if c == '\'' {
            if at(i + 1) == '\\' {
                i += 3;
                while i < chars.len() && chars[i] != '\'' {
                    i += 1;
                }
                i += 1;
            } else if at(i + 2) == '\'' {
                i += 3;
            } else {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    lines.len()
}

fn raw_prefix_ok(chars: &[char], i: usize) -> bool {
    let before = |k: usize| i.checked_sub(k).map_or('\0', |j| chars[j]);
    !is_ident(before(1)) || (matches!(before(1), 'b' | 'c') && !is_ident(before(2)))
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

#[test]
fn lexer_ignores_strings_and_counts_every_comment_form() {
    let source = concat!(
        "let a = \"s3://bucket\"; let b = r#\"// \"quoted\" \"#; let q = '\\'';\n",
        "let g = br\"\\\"; let h = b'\"'; // trailing after raw\n",
        "let k = cr\"\\\"; // trailing after a C raw string\n",
        "let c = '/'; let d: &'static str = \"\\\" // still a string\";\n",
        "fn f() {} // trailing\n",
        "/// doc\n",
        "//! inner doc\n",
        "/* block\n",
        "   /* nested */ still */ let e = 1;\n",
        "let f = 2;\n",
    );
    assert_eq!(rust_comment_lines(source), 7);
}

#[test]
fn hash_counter_skips_the_shebang() {
    assert_eq!(
        hash_comment_lines("#!/bin/sh\n# one\necho \"#x\"\n  # two\n"),
        2
    );
}
