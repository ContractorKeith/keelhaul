use crate::repo::Error;
use globset::{GlobBuilder, GlobMatcher};
use std::path::Path;

pub struct Patterns(Vec<Rule>);
struct Rule {
    matcher: GlobMatcher,
    basename: bool,
    directory: bool,
    include: bool,
}
impl Patterns {
    pub fn new(patterns: &[String]) -> Result<Self, Error> {
        let mut rules = Vec::new();
        for raw in patterns {
            let normalized = gitignore_pattern(raw);
            if normalized.is_empty() {
                continue;
            }
            let raw = &normalized;
            let (include, text) = raw
                .strip_prefix('!')
                .map_or((false, raw.as_str()), |p| (true, p));
            let directory = text.ends_with('/');
            let text = text.trim_end_matches('/');
            let anchored = text.starts_with('/');
            let text = text.strip_prefix('/').unwrap_or(text);
            if text.is_empty() {
                return Err(Error::Refused(format!("empty exclude pattern: {raw}")));
            }
            let glob = GlobBuilder::new(text)
                .literal_separator(true)
                .backslash_escape(true)
                .build()
                .map_err(|e| Error::Refused(format!("exclude {raw}: {e}")))?;
            rules.push(Rule {
                matcher: glob.compile_matcher(),
                basename: !anchored && !text.contains('/'),
                directory,
                include,
            });
        }
        Ok(Self(rules))
    }
    pub fn excluded(&self, relative: &Path, directory: bool) -> bool {
        let mut excluded = false;
        for rule in &self.0 {
            if rule.directory && !directory {
                continue;
            }
            let candidate = if rule.basename {
                relative.file_name().map(Path::new).unwrap_or(relative)
            } else {
                relative
            };
            if rule.matcher.is_match(candidate) {
                excluded = !rule.include;
            }
        }
        excluded
    }
}

// Gitignore treats braces literally and discards only unescaped trailing spaces.
fn gitignore_pattern(raw: &str) -> String {
    let mut end = raw.len();
    while raw.as_bytes().get(end.wrapping_sub(1)) == Some(&b' ') {
        let slashes = raw.as_bytes()[..end - 1]
            .iter()
            .rev()
            .take_while(|b| **b == b'\\')
            .count();
        if slashes % 2 == 1 {
            break;
        }
        end -= 1;
    }
    let mut result = String::new();
    let mut escaped = false;
    for c in raw[..end].chars() {
        if !escaped && matches!(c, '{' | '}') {
            result.push('\\');
        }
        result.push(c);
        escaped = !escaped && c == '\\';
    }
    result
}
