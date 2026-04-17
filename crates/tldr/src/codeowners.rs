use std::path::Path;

pub struct CodeOwners {
    patterns: Vec<(String, Vec<String>)>,
}

impl CodeOwners {
    pub fn parse(text: &str) -> CodeOwners {
        let mut patterns = Vec::new();
        for line in text.lines() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() { continue; }
            let mut parts = line.split_whitespace();
            let pat = match parts.next() { Some(p) => p.to_string(), None => continue };
            let owners: Vec<String> = parts.map(|s| s.to_string()).collect();
            patterns.push((pat, owners));
        }
        CodeOwners { patterns }
    }

    pub fn owners_for(&self, path: &str) -> Vec<&str> {
        let mut last: Option<&Vec<String>> = None;
        for (pat, owners) in &self.patterns {
            if matches_pattern(pat, path) {
                last = Some(owners);
            }
        }
        last.map(|o| o.iter().map(|s| s.as_str()).collect()).unwrap_or_default()
    }

    pub fn load_from_repo(root: &Path) -> Option<CodeOwners> {
        for p in [".github/CODEOWNERS", "CODEOWNERS", "docs/CODEOWNERS"] {
            let f = root.join(p);
            if let Ok(s) = std::fs::read_to_string(&f) {
                return Some(Self::parse(&s));
            }
        }
        None
    }
}

/// Gitignore-style matching for CODEOWNERS.
/// - Leading `/` anchors to repo root.
/// - Trailing `/` matches directory (any file inside).
/// - `*` matches any chars except `/`.
/// - `**` matches any chars including `/`.
/// - Pattern without `/` (besides trailing) matches by basename at any depth.
fn matches_pattern(pat: &str, path: &str) -> bool {
    let path = path.trim_start_matches('/');
    let mut pat = pat.to_string();
    let dir_only = pat.ends_with('/');
    if dir_only { pat.pop(); }

    let anchored = pat.starts_with('/');
    if anchored { pat.remove(0); }

    // Pattern with no slash => match basename at any depth (unless anchored).
    let has_slash = pat.contains('/');

    if dir_only {
        // directory match: path must be under pat/
        if anchored || has_slash {
            return path == pat || path.starts_with(&format!("{pat}/")) || glob_match_prefix(&pat, path);
        } else {
            for seg_end in path.match_indices('/') {
                let seg = &path[..seg_end.0];
                let base = seg.rsplit('/').next().unwrap_or(seg);
                if glob_match(&pat, base) { return true; }
            }
            return false;
        }
    }

    if anchored || has_slash {
        glob_match(&pat, path)
    } else {
        // basename match at any depth
        for part in path.split('/') {
            if glob_match(&pat, part) { return true; }
        }
        glob_match(&pat, path)
    }
}

fn glob_match_prefix(pat: &str, path: &str) -> bool {
    // Match when path starts with a prefix that matches pat + '/'
    if let Some(idx) = path.find('/') {
        let (head, _) = path.split_at(idx);
        if glob_match(pat, head) { return true; }
    }
    glob_match(pat, path)
}

fn glob_match(pat: &str, s: &str) -> bool {
    glob_rec(pat.as_bytes(), s.as_bytes())
}

fn glob_rec(pat: &[u8], s: &[u8]) -> bool {
    let mut pi = 0;
    let mut si = 0;
    let mut star: Option<(usize, usize, bool)> = None; // (pi_after_star, si_backtrack, double)
    while si < s.len() {
        if pi < pat.len() {
            let c = pat[pi];
            if c == b'*' {
                let double = pi + 1 < pat.len() && pat[pi + 1] == b'*';
                let after = if double { pi + 2 } else { pi + 1 };
                star = Some((after, si, double));
                pi = after;
                continue;
            } else if c == b'?' {
                if s[si] != b'/' { pi += 1; si += 1; continue; }
            } else if c == s[si] {
                pi += 1; si += 1; continue;
            }
        }
        if let Some((after, back, double)) = star {
            if !double && s[back] == b'/' {
                // single star can't consume slash beyond backtrack
                return false;
            }
            let new_back = back + 1;
            if new_back > s.len() { return false; }
            if !double && si < s.len() && s[si] == b'/' && new_back > si {
                return false;
            }
            star = Some((after, new_back, double));
            pi = after;
            si = new_back;
        } else {
            return false;
        }
    }
    while pi < pat.len() && pat[pi] == b'*' { pi += 1; }
    pi == pat.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic() {
        let co = CodeOwners::parse("* @alice\n/src/ @bob\n*.rs @carol\n# comment\ndocs/ @dave @team/eng\n");
        assert_eq!(co.owners_for("README.md"), vec!["@alice"]);
        assert_eq!(co.owners_for("src/main.rs"), vec!["@carol"]);
        assert_eq!(co.owners_for("docs/guide.md"), vec!["@dave", "@team/eng"]);
    }

    #[test]
    fn last_match_wins() {
        let co = CodeOwners::parse("* @a\n*.rs @b\n/src/special.rs @c\n");
        assert_eq!(co.owners_for("src/special.rs"), vec!["@c"]);
        assert_eq!(co.owners_for("other.rs"), vec!["@b"]);
        assert_eq!(co.owners_for("README.md"), vec!["@a"]);
    }

    #[test]
    fn team_and_email_owners() {
        let co = CodeOwners::parse("api/ @org/api-team dev@example.com\n");
        let o = co.owners_for("api/foo.ts");
        assert!(o.contains(&"@org/api-team"));
        assert!(o.contains(&"dev@example.com"));
    }

    #[test]
    fn anchored_vs_recursive() {
        let co = CodeOwners::parse("/tools/ @root-only\ntools/ @any-depth\n");
        assert_eq!(co.owners_for("tools/x.py"), vec!["@any-depth"]);
        assert_eq!(co.owners_for("vendor/tools/x.py"), vec!["@any-depth"]);
    }

    #[test]
    fn double_star_matches_recursively() {
        // ** should match across directories
        assert!(matches_pattern("**/test_foo.py", "a/b/test_foo.py"));
    }

    #[test]
    fn basename_pattern_any_depth() {
        assert!(matches_pattern("*.md", "docs/guide.md"));
        assert!(matches_pattern("*.md", "README.md"));
        assert!(!matches_pattern("*.md", "main.rs"));
    }

    #[test]
    fn no_match_returns_empty() {
        let co = CodeOwners::parse("/only/ @x\n");
        assert!(co.owners_for("other/file.rs").is_empty());
    }

    #[test]
    fn comments_ignored() {
        let co = CodeOwners::parse("# this is a comment\n* @a # inline\n");
        assert_eq!(co.owners_for("x.md"), vec!["@a"]);
    }
}
