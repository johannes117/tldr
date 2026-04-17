use anyhow::{anyhow, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Diff {
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileDiff {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String, // modified|added|deleted|renamed
    #[serde(default)]
    pub similarity: Option<u32>,
    pub hunks: Vec<Hunk>,
    #[serde(default)]
    pub is_generated: bool,
    #[serde(default)]
    pub is_large: bool,
    #[serde(default)]
    pub is_binary: bool,
    #[serde(default)]
    pub is_image: bool,
    #[serde(default)]
    pub stats: Stats,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<ImagePayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Stats {
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImagePayload {
    pub old_data_url: Option<String>,
    pub new_data_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hunk {
    pub header: String,
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Line {
    pub kind: String, // "add" | "del" | "ctx"
    pub content: String,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moved: Option<MovedLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MovedLink {
    pub kind: String, // "moved-from" | "moved-to"
    pub path: String,
    pub line: u32,
}

const LARGE_THRESHOLD: usize = 5000;
const MOVE_MIN_RUN: usize = 5;

pub fn compute(repo: &Path, base: &str, head: &str) -> Result<Diff> {
    compute_opts(repo, base, head, false, None)
}

pub fn compute_file(repo: &Path, base: &str, head: &str, path: &str) -> Result<Option<FileDiff>> {
    let d = compute_opts(repo, base, head, true, Some(path))?;
    Ok(d.files.into_iter().find(|f| f.path == path))
}

fn compute_opts(repo: &Path, base: &str, head: &str, expand: bool, only: Option<&str>) -> Result<Diff> {
    let mut args = vec![
        "-C",
        repo.to_str().unwrap(),
        "diff",
        "--no-color",
        "-M50%",
        &format!("{base}..{head}"),
    ]
    .into_iter()
    .map(String::from)
    .collect::<Vec<_>>();
    if let Some(p) = only {
        args.push("--".into());
        args.push(p.to_string());
    }
    let out = Command::new("git").args(&args).output()?;
    if !out.status.success() {
        return Err(anyhow!("git diff: {}", String::from_utf8_lossy(&out.stderr)));
    }
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    let mut d = parse_unified(&text);

    // Annotate generated + large + binary/image
    for f in d.files.iter_mut() {
        f.is_generated = is_generated_path(&f.path)
            || is_generated_path(f.old_path.as_deref().unwrap_or(""))
            || first_lines_generated(repo, head, &f.path).unwrap_or(false);
        let total_changed = f.hunks.iter().map(|h| h.lines.iter().filter(|l| l.kind != "ctx").count()).sum::<usize>();
        f.stats.added = f.hunks.iter().flat_map(|h| h.lines.iter()).filter(|l| l.kind == "add").count() as u32;
        f.stats.removed = f.hunks.iter().flat_map(|h| h.lines.iter()).filter(|l| l.kind == "del").count() as u32;
        if !expand && total_changed > LARGE_THRESHOLD {
            f.is_large = true;
            f.hunks.clear();
        }
        if is_image_path(&f.path) {
            f.is_image = true;
            f.is_binary = true;
            f.image = Some(ImagePayload {
                old_data_url: blob_data_url(repo, base, f.old_path.as_deref().unwrap_or(&f.path)).ok(),
                new_data_url: blob_data_url(repo, head, &f.path).ok(),
            });
        }
    }

    // Detect binary files from git output markers
    for bp in parse_binary_paths(&text) {
        if let Some(f) = d.files.iter_mut().find(|f| f.path == bp) {
            f.is_binary = true;
        }
    }

    detect_moves(&mut d);
    Ok(d)
}

fn is_generated_path(p: &str) -> bool {
    if p.is_empty() {
        return false;
    }
    let base = p.rsplit('/').next().unwrap_or(p);
    matches!(base, "package-lock.json" | "yarn.lock" | "pnpm-lock.yaml" | "Cargo.lock")
        || p.ends_with(".pb.go")
        || base.contains(".generated.")
        || p.ends_with(".min.js")
        || p.ends_with(".min.css")
}

fn first_lines_generated(repo: &Path, sha: &str, path: &str) -> Result<bool> {
    let out = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "show", &format!("{sha}:{path}")])
        .output()?;
    if !out.status.success() {
        return Ok(false);
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Ok(text.lines().take(5).any(|l| l.contains("@generated")))
}

fn is_image_path(p: &str) -> bool {
    let lower = p.to_lowercase();
    ["png", "jpg", "jpeg", "gif", "webp", "svg"]
        .iter()
        .any(|ext| lower.ends_with(&format!(".{ext}")))
}

fn image_mime(p: &str) -> &'static str {
    let lower = p.to_lowercase();
    if lower.ends_with(".png") { "image/png" }
    else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") { "image/jpeg" }
    else if lower.ends_with(".gif") { "image/gif" }
    else if lower.ends_with(".webp") { "image/webp" }
    else if lower.ends_with(".svg") { "image/svg+xml" }
    else { "application/octet-stream" }
}

fn blob_data_url(repo: &Path, sha: &str, path: &str) -> Result<String> {
    let out = Command::new("git")
        .args(["-C", repo.to_str().unwrap(), "show", &format!("{sha}:{path}")])
        .output()?;
    if !out.status.success() {
        return Err(anyhow!("git show failed"));
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&out.stdout);
    Ok(format!("data:{};base64,{}", image_mime(path), b64))
}

fn parse_binary_paths(text: &str) -> Vec<String> {
    // "Binary files a/foo and b/foo differ"
    let mut out = vec![];
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("Binary files ") {
            if let Some(idx) = rest.find(" and b/") {
                let tail = &rest[idx + " and b/".len()..];
                if let Some(p) = tail.strip_suffix(" differ") {
                    out.push(p.to_string());
                }
            }
        }
    }
    out
}

pub fn parse_unified(text: &str) -> Diff {
    let mut files: Vec<FileDiff> = vec![];
    let mut cur_file: Option<FileDiff> = None;
    let mut cur_hunk: Option<Hunk> = None;
    let mut old_ln: u32 = 0;
    let mut new_ln: u32 = 0;

    for line in text.lines() {
        if line.starts_with("diff --git ") {
            if let Some(h) = cur_hunk.take() { if let Some(f) = cur_file.as_mut() { f.hunks.push(h); } }
            if let Some(f) = cur_file.take() { files.push(f); }
            let parts: Vec<&str> = line.splitn(4, ' ').collect();
            let path = parts.get(3).unwrap_or(&"").trim_start_matches("b/").to_string();
            cur_file = Some(FileDiff { path, status: "modified".into(), ..Default::default() });
        } else if line.starts_with("new file mode") {
            if let Some(f) = cur_file.as_mut() { f.status = "added".into(); }
        } else if line.starts_with("deleted file mode") {
            if let Some(f) = cur_file.as_mut() { f.status = "deleted".into(); }
        } else if line.starts_with("rename from ") {
            if let Some(f) = cur_file.as_mut() {
                f.old_path = Some(line.trim_start_matches("rename from ").to_string());
                f.status = "renamed".into();
            }
        } else if line.starts_with("rename to ") {
            if let Some(f) = cur_file.as_mut() {
                f.path = line.trim_start_matches("rename to ").to_string();
            }
        } else if let Some(rest) = line.strip_prefix("similarity index ") {
            if let Some(f) = cur_file.as_mut() {
                let n = rest.trim_end_matches('%').parse::<u32>().ok();
                f.similarity = n;
            }
        } else if line.starts_with("+++ b/") {
            if let Some(f) = cur_file.as_mut() { f.path = line.trim_start_matches("+++ b/").to_string(); }
        } else if line.starts_with("--- a/") {
            if let Some(f) = cur_file.as_mut() {
                if f.old_path.is_none() { f.old_path = Some(line.trim_start_matches("--- a/").to_string()); }
            }
        } else if line.starts_with("@@") {
            if let Some(h) = cur_hunk.take() { if let Some(f) = cur_file.as_mut() { f.hunks.push(h); } }
            let (os, ns) = parse_hunk_header(line);
            old_ln = os; new_ln = ns;
            cur_hunk = Some(Hunk { header: line.to_string(), old_start: os, new_start: ns, lines: vec![] });
        } else if let Some(h) = cur_hunk.as_mut() {
            if let Some(rest) = line.strip_prefix('+') {
                h.lines.push(Line { kind: "add".into(), content: rest.to_string(), old_line: None, new_line: Some(new_ln), moved: None });
                new_ln += 1;
            } else if let Some(rest) = line.strip_prefix('-') {
                h.lines.push(Line { kind: "del".into(), content: rest.to_string(), old_line: Some(old_ln), new_line: None, moved: None });
                old_ln += 1;
            } else if let Some(rest) = line.strip_prefix(' ') {
                h.lines.push(Line { kind: "ctx".into(), content: rest.to_string(), old_line: Some(old_ln), new_line: Some(new_ln), moved: None });
                old_ln += 1; new_ln += 1;
            }
        }
    }
    if let Some(h) = cur_hunk.take() { if let Some(f) = cur_file.as_mut() { f.hunks.push(h); } }
    if let Some(f) = cur_file.take() { files.push(f); }
    Diff { files }
}

fn parse_hunk_header(s: &str) -> (u32, u32) {
    let mut old_s = 0u32; let mut new_s = 0u32;
    let parts: Vec<&str> = s.split(' ').collect();
    for p in parts {
        if let Some(rest) = p.strip_prefix('-') {
            old_s = rest.split(',').next().and_then(|x| x.parse().ok()).unwrap_or(0);
        } else if let Some(rest) = p.strip_prefix('+') {
            new_s = rest.split(',').next().and_then(|x| x.parse().ok()).unwrap_or(0);
        }
    }
    (old_s, new_s)
}

// Moved-code detection: find contiguous runs of >=5 matching hash lines
// where each was a deletion in one file and an addition in another (or same).
#[derive(Clone)]
struct LocRef {
    file_idx: usize,
    hunk_idx: usize,
    line_idx: usize,
    line_no: u32,
    path: String,
}

fn line_hash(s: &str) -> Option<u64> {
    let t = s.trim();
    if t.is_empty() || t.len() < 3 {
        return None;
    }
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    Some(h.finish())
}

fn detect_moves(d: &mut Diff) {
    // collect sequences of adds and dels per file
    let mut adds: HashMap<u64, Vec<LocRef>> = HashMap::new();
    let mut dels: HashMap<u64, Vec<LocRef>> = HashMap::new();

    for (fi, f) in d.files.iter().enumerate() {
        let path_del = f.old_path.clone().unwrap_or_else(|| f.path.clone());
        for (hi, h) in f.hunks.iter().enumerate() {
            for (li, l) in h.lines.iter().enumerate() {
                if let Some(hash) = line_hash(&l.content) {
                    if l.kind == "add" {
                        adds.entry(hash).or_default().push(LocRef {
                            file_idx: fi, hunk_idx: hi, line_idx: li,
                            line_no: l.new_line.unwrap_or(0), path: f.path.clone(),
                        });
                    } else if l.kind == "del" {
                        dels.entry(hash).or_default().push(LocRef {
                            file_idx: fi, hunk_idx: hi, line_idx: li,
                            line_no: l.old_line.unwrap_or(0), path: path_del.clone(),
                        });
                    }
                }
            }
        }
    }

    // For each del line find an add with same hash; then extend run.
    // Flat list of (add_pos, del_pos) pairs; pos = (fi, hi, li) flattened
    let get_add_hash = |d: &Diff, fi: usize, hi: usize, li: usize| -> Option<u64> {
        let l = d.files.get(fi)?.hunks.get(hi)?.lines.get(li)?;
        if l.kind == "add" { line_hash(&l.content) } else { None }
    };
    let get_del_hash = |d: &Diff, fi: usize, hi: usize, li: usize| -> Option<u64> {
        let l = d.files.get(fi)?.hunks.get(hi)?.lines.get(li)?;
        if l.kind == "del" { line_hash(&l.content) } else { None }
    };

    // iterate over dels
    let del_positions: Vec<(usize, usize, usize)> = d
        .files
        .iter()
        .enumerate()
        .flat_map(|(fi, f)| {
            f.hunks.iter().enumerate().flat_map(move |(hi, h)| {
                h.lines.iter().enumerate().filter_map(move |(li, l)| {
                    if l.kind == "del" { Some((fi, hi, li)) } else { None }
                })
            })
        })
        .collect();

    let mut marks: Vec<(usize, usize, usize, MovedLink)> = vec![];
    let mut consumed: std::collections::HashSet<(usize, usize, usize)> = Default::default();

    for (fi, hi, li) in del_positions {
        if consumed.contains(&(fi, hi, li)) { continue; }
        let hash = match get_del_hash(&d, fi, hi, li) { Some(h) => h, None => continue };
        let candidates = match adds.get(&hash) { Some(v) => v.clone(), None => continue };
        for cand in candidates {
            // try to extend run
            let mut run = 1usize;
            loop {
                let dpos = (fi, hi, li + run);
                let apos = (cand.file_idx, cand.hunk_idx, cand.line_idx + run);
                let dh = get_del_hash(&d, dpos.0, dpos.1, dpos.2);
                let ah = get_add_hash(&d, apos.0, apos.1, apos.2);
                match (dh, ah) {
                    (Some(a), Some(b)) if a == b && !consumed.contains(&dpos) => run += 1,
                    _ => break,
                }
            }
            if run >= MOVE_MIN_RUN {
                let del_path = d.files[fi].old_path.clone().unwrap_or_else(|| d.files[fi].path.clone());
                let del_line_no = d.files[fi].hunks[hi].lines[li].old_line.unwrap_or(0);
                let add_path = cand.path.clone();
                let add_line_no = cand.line_no;
                for k in 0..run {
                    let dpos = (fi, hi, li + k);
                    let apos = (cand.file_idx, cand.hunk_idx, cand.line_idx + k);
                    consumed.insert(dpos);
                    consumed.insert(apos);
                    marks.push((dpos.0, dpos.1, dpos.2, MovedLink {
                        kind: "moved-to".into(), path: add_path.clone(), line: add_line_no,
                    }));
                    marks.push((apos.0, apos.1, apos.2, MovedLink {
                        kind: "moved-from".into(), path: del_path.clone(), line: del_line_no,
                    }));
                }
                break;
            }
        }
    }

    for (fi, hi, li, m) in marks {
        if let Some(l) = d.files.get_mut(fi).and_then(|f| f.hunks.get_mut(hi)).and_then(|h| h.lines.get_mut(li)) {
            l.moved = Some(m);
        }
    }

    let _ = dels; // silence
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE_DIFF: &str = "diff --git a/foo.txt b/foo.txt\nindex 111..222 100644\n--- a/foo.txt\n+++ b/foo.txt\n@@ -1,3 +1,4 @@\n line one\n-line two\n+line two changed\n+new line\n line three\n";

    const RENAME_DIFF: &str = "diff --git a/old.txt b/new.txt\nsimilarity index 85%\nrename from old.txt\nrename to new.txt\n--- a/old.txt\n+++ b/new.txt\n@@ -1,2 +1,2 @@\n alpha\n-beta\n+gamma\n";

    const BINARY_DIFF: &str = "diff --git a/img.bin b/img.bin\nindex abc..def 100644\nBinary files a/img.bin and b/img.bin differ\n";

    const ADD_DEL_DIFF: &str = "diff --git a/added.rs b/added.rs\nnew file mode 100644\n--- /dev/null\n+++ b/added.rs\n@@ -0,0 +1,2 @@\n+hello\n+world\ndiff --git a/gone.rs b/gone.rs\ndeleted file mode 100644\n--- a/gone.rs\n+++ /dev/null\n@@ -1,1 +0,0 @@\n-bye\n";

    #[test]
    fn parses_simple_diff() {
        let d = parse_unified(SIMPLE_DIFF);
        assert_eq!(d.files.len(), 1);
        let f = &d.files[0];
        assert_eq!(f.path, "foo.txt");
        assert_eq!(f.status, "modified");
        assert_eq!(f.hunks.len(), 1);
        let h = &f.hunks[0];
        assert_eq!(h.old_start, 1);
        assert_eq!(h.new_start, 1);
        let adds = h.lines.iter().filter(|l| l.kind == "add").count();
        let dels = h.lines.iter().filter(|l| l.kind == "del").count();
        assert_eq!(adds, 2);
        assert_eq!(dels, 1);
    }

    #[test]
    fn parses_rename() {
        let d = parse_unified(RENAME_DIFF);
        assert_eq!(d.files.len(), 1);
        let f = &d.files[0];
        assert_eq!(f.status, "renamed");
        assert_eq!(f.path, "new.txt");
        assert_eq!(f.old_path.as_deref(), Some("old.txt"));
    }

    #[test]
    fn parses_binary_paths() {
        let paths = parse_binary_paths(BINARY_DIFF);
        assert_eq!(paths, vec!["img.bin"]);
    }

    #[test]
    fn parses_add_and_delete() {
        let d = parse_unified(ADD_DEL_DIFF);
        assert_eq!(d.files.len(), 2);
        assert_eq!(d.files[0].status, "added");
        assert_eq!(d.files[1].status, "deleted");
    }

    #[test]
    fn is_generated_covers_common() {
        assert!(is_generated_path("path/to/package-lock.json"));
        assert!(is_generated_path("Cargo.lock"));
        assert!(is_generated_path("foo.pb.go"));
        assert!(is_generated_path("foo.min.js"));
        assert!(is_generated_path("bar.generated.ts"));
        assert!(!is_generated_path("src/main.rs"));
        assert!(!is_generated_path(""));
    }

    #[test]
    fn image_detection() {
        assert!(is_image_path("logo.PNG"));
        assert!(is_image_path("a/b/c.svg"));
        assert!(!is_image_path("main.rs"));
        assert_eq!(image_mime("x.png"), "image/png");
        assert_eq!(image_mime("x.jpeg"), "image/jpeg");
        assert_eq!(image_mime("x.svg"), "image/svg+xml");
        assert_eq!(image_mime("x.bin"), "application/octet-stream");
    }

    #[test]
    fn hunk_header_parses() {
        assert_eq!(parse_hunk_header("@@ -10,5 +20,7 @@ ctx"), (10, 20));
        assert_eq!(parse_hunk_header("@@ -1 +1 @@"), (1, 1));
    }

    #[test]
    fn move_detection_links_runs() {
        // 5 identical lines deleted from a.rs and added to b.rs
        let text = "\
diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -1,5 +0,0 @@\n-line alpha one\n-line beta two\n-line gamma three\n-line delta four\n-line epsilon five\ndiff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -0,0 +1,5 @@\n+line alpha one\n+line beta two\n+line gamma three\n+line delta four\n+line epsilon five\n";
        let mut d = parse_unified(text);
        detect_moves(&mut d);
        let any_moved = d.files.iter().flat_map(|f| f.hunks.iter()).flat_map(|h| h.lines.iter()).any(|l| l.moved.is_some());
        assert!(any_moved, "expected moves detected");
    }

    #[test]
    fn large_file_placeholder() {
        // Synthesize large diff (>5000 changed lines)
        let mut s = String::from("diff --git a/big.txt b/big.txt\n--- a/big.txt\n+++ b/big.txt\n@@ -1,6000 +1,6000 @@\n");
        for i in 0..6000 { s.push_str(&format!("-old line {i}\n+new line {i}\n")); }
        let d = parse_unified(&s);
        assert_eq!(d.files.len(), 1);
        // Simulate large marking from compute_opts
        let total_changed: usize = d.files[0].hunks.iter().map(|h| h.lines.iter().filter(|l| l.kind != "ctx").count()).sum();
        assert!(total_changed > LARGE_THRESHOLD);
    }
}
