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
