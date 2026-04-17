use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CoverageFormat {
    Lcov,
    Cobertura,
    GoCover,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Hit {
    Covered,
    Uncovered,
    NotInstrumented,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileCoverage {
    pub path: String,
    pub lines: HashMap<u32, Hit>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RawCoverage {
    pub format: CoverageFormat,
    pub text: String,
}

pub fn parse_lcov(text: &str) -> Result<HashMap<String, FileCoverage>> {
    let mut out: HashMap<String, FileCoverage> = HashMap::new();
    let mut cur: Option<FileCoverage> = None;
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("SF:") {
            cur = Some(FileCoverage { path: rest.to_string(), lines: HashMap::new() });
        } else if let Some(rest) = line.strip_prefix("DA:") {
            if let Some(fc) = cur.as_mut() {
                let mut it = rest.splitn(3, ',');
                let ln: u32 = it.next().unwrap_or("0").parse().unwrap_or(0);
                let count: u64 = it.next().unwrap_or("0").parse().unwrap_or(0);
                if ln > 0 {
                    fc.lines.insert(ln, if count > 0 { Hit::Covered } else { Hit::Uncovered });
                }
            }
        } else if line == "end_of_record" {
            if let Some(fc) = cur.take() {
                out.insert(fc.path.clone(), fc);
            }
        }
    }
    if let Some(fc) = cur.take() {
        out.insert(fc.path.clone(), fc);
    }
    Ok(out)
}

pub fn parse_cobertura(xml: &str) -> Result<HashMap<String, FileCoverage>> {
    use quick_xml::events::Event;
    use quick_xml::Reader;
    let mut reader = Reader::from_str(xml);
    reader.config_mut().trim_text(true);
    let mut out: HashMap<String, FileCoverage> = HashMap::new();
    let mut cur_file: Option<FileCoverage> = None;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                let name = e.name();
                let tag = std::str::from_utf8(name.as_ref()).unwrap_or("").to_string();
                if tag == "class" {
                    let mut filename = String::new();
                    for a in e.attributes().flatten() {
                        if a.key.as_ref() == b"filename" {
                            filename = String::from_utf8_lossy(&a.value).to_string();
                        }
                    }
                    if !filename.is_empty() {
                        if let Some(prev) = cur_file.take() {
                            out.entry(prev.path.clone()).or_insert(prev);
                        }
                        cur_file = Some(FileCoverage { path: filename, lines: HashMap::new() });
                    }
                } else if tag == "line" {
                    let mut num: u32 = 0;
                    let mut hits: u64 = 0;
                    for a in e.attributes().flatten() {
                        match a.key.as_ref() {
                            b"number" => num = String::from_utf8_lossy(&a.value).parse().unwrap_or(0),
                            b"hits" => hits = String::from_utf8_lossy(&a.value).parse().unwrap_or(0),
                            _ => {}
                        }
                    }
                    if let Some(fc) = cur_file.as_mut() {
                        if num > 0 {
                            fc.lines.insert(num, if hits > 0 { Hit::Covered } else { Hit::Uncovered });
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                if e.name().as_ref() == b"class" {
                    if let Some(fc) = cur_file.take() {
                        out.entry(fc.path.clone()).or_insert(fc);
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("xml parse: {e}")),
            _ => {}
        }
        buf.clear();
    }
    if let Some(fc) = cur_file.take() {
        out.entry(fc.path.clone()).or_insert(fc);
    }
    Ok(out)
}

pub fn parse_go_cover(text: &str) -> Result<HashMap<String, FileCoverage>> {
    let mut out: HashMap<String, FileCoverage> = HashMap::new();
    for (i, line) in text.lines().enumerate() {
        if i == 0 && line.starts_with("mode:") {
            continue;
        }
        if line.trim().is_empty() { continue; }
        // file.go:startLine.startCol,endLine.endCol numStmts count
        let (loc, rest) = match line.split_once(' ') {
            Some(v) => v,
            None => continue,
        };
        let mut rest_parts = rest.split_whitespace();
        let _num_stmts = rest_parts.next().unwrap_or("0");
        let count: u64 = rest_parts.next().unwrap_or("0").parse().unwrap_or(0);
        let (file, span) = match loc.split_once(':') {
            Some(v) => v,
            None => continue,
        };
        let (start, end) = match span.split_once(',') {
            Some(v) => v,
            None => continue,
        };
        let start_line: u32 = start.split('.').next().unwrap_or("0").parse().unwrap_or(0);
        let end_line: u32 = end.split('.').next().unwrap_or("0").parse().unwrap_or(0);
        let entry = out.entry(file.to_string()).or_insert_with(|| FileCoverage {
            path: file.to_string(),
            lines: HashMap::new(),
        });
        for ln in start_line..=end_line {
            if ln == 0 { continue; }
            let hit = if count > 0 { Hit::Covered } else { Hit::Uncovered };
            entry.lines.entry(ln).and_modify(|e| {
                if *e != Hit::Covered && hit == Hit::Covered { *e = Hit::Covered; }
            }).or_insert(hit);
        }
    }
    Ok(out)
}

pub fn parse_any(text: &str, hint_name: Option<&str>) -> Result<HashMap<String, FileCoverage>> {
    if let Some(n) = hint_name {
        let l = n.to_lowercase();
        if l.ends_with(".xml") || l.contains("cobertura") {
            return parse_cobertura(text);
        }
        if l.ends_with(".out") || l.contains("gocover") || l.contains("coverage.out") {
            return parse_go_cover(text);
        }
        if l.ends_with(".info") || l.contains("lcov") {
            return parse_lcov(text);
        }
    }
    // sniff
    let head = text.trim_start();
    if head.starts_with('<') {
        parse_cobertura(text)
    } else if head.starts_with("mode:") {
        parse_go_cover(text)
    } else {
        parse_lcov(text)
    }
}

pub fn detect_local(worktree: &Path) -> Option<(CoverageFormat, String, String)> {
    let candidates: &[(&str, CoverageFormat)] = &[
        ("coverage/lcov.info", CoverageFormat::Lcov),
        ("lcov.info", CoverageFormat::Lcov),
        ("coverage.lcov", CoverageFormat::Lcov),
        ("coverage.xml", CoverageFormat::Cobertura),
        ("coverage/cobertura-coverage.xml", CoverageFormat::Cobertura),
        ("coverage.out", CoverageFormat::GoCover),
    ];
    for (rel, fmt) in candidates {
        let p = worktree.join(rel);
        if p.exists() {
            if let Ok(s) = std::fs::read_to_string(&p) {
                return Some((*fmt, rel.to_string(), s));
            }
        }
    }
    None
}

pub async fn fetch_ci_artifact(
    token: &str,
    owner: &str,
    repo: &str,
    head_sha: &str,
) -> Result<Option<RawCoverage>> {
    let http = reqwest::Client::builder()
        .user_agent("tldr-cli")
        .build()?;
    let runs_url = format!(
        "https://api.github.com/repos/{owner}/{repo}/actions/runs?head_sha={head_sha}&per_page=20"
    );
    let runs: serde_json::Value = http
        .get(&runs_url)
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let empty = vec![];
    let runs = runs.get("workflow_runs").and_then(|v| v.as_array()).unwrap_or(&empty);
    for run in runs {
        let run_id = match run.get("id").and_then(|v| v.as_i64()) {
            Some(v) => v,
            None => continue,
        };
        let art_url = format!(
            "https://api.github.com/repos/{owner}/{repo}/actions/runs/{run_id}/artifacts?per_page=100"
        );
        let arts: serde_json::Value = match http
            .get(&art_url)
            .header("Accept", "application/vnd.github+json")
            .header("Authorization", format!("Bearer {token}"))
            .send()
            .await
        {
            Ok(r) => match r.error_for_status() {
                Ok(r) => r.json().await.unwrap_or(serde_json::Value::Null),
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        let empty2 = vec![];
        let list = arts.get("artifacts").and_then(|v| v.as_array()).unwrap_or(&empty2);
        for a in list {
            let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("").to_lowercase();
            let looks_cov = name.contains("coverage") || name.contains("lcov") || name.contains("cobertura");
            if !looks_cov { continue; }
            let archive_url = match a.get("archive_download_url").and_then(|v| v.as_str()) {
                Some(v) => v,
                None => continue,
            };
            let resp = match http
                .get(archive_url)
                .header("Accept", "application/vnd.github+json")
                .header("Authorization", format!("Bearer {token}"))
                .send()
                .await
            {
                Ok(r) => match r.error_for_status() { Ok(r) => r, Err(_) => continue },
                Err(_) => continue,
            };
            let bytes = match resp.bytes().await { Ok(b) => b, Err(_) => continue };
            if let Some(raw) = extract_coverage_from_zip(&bytes) {
                return Ok(Some(raw));
            }
        }
    }
    Ok(None)
}

fn extract_coverage_from_zip(bytes: &[u8]) -> Option<RawCoverage> {
    // Minimal zip-less approach: try to interpret as zip via quick sniff.
    // We use a tiny pure-rust approach: fall back to scanning as text for simple cases.
    // Since `zip` crate isn't in Cargo.toml, try treating the bytes as raw file if heuristic works.
    if let Ok(s) = std::str::from_utf8(bytes) {
        if s.contains("SF:") && s.contains("end_of_record") {
            return Some(RawCoverage { format: CoverageFormat::Lcov, text: s.to_string() });
        }
        if s.trim_start().starts_with("<?xml") && s.contains("<coverage") {
            return Some(RawCoverage { format: CoverageFormat::Cobertura, text: s.to_string() });
        }
        if s.starts_with("mode:") {
            return Some(RawCoverage { format: CoverageFormat::GoCover, text: s.to_string() });
        }
    }
    // Rudimentary zip central directory parse: find local file headers (PK\x03\x04) and stored (method 0) entries.
    let mut i = 0usize;
    while i + 30 <= bytes.len() {
        if &bytes[i..i + 4] == b"PK\x03\x04" {
            let method = u16::from_le_bytes([bytes[i + 8], bytes[i + 9]]);
            let comp_size = u32::from_le_bytes([bytes[i + 18], bytes[i + 19], bytes[i + 20], bytes[i + 21]]) as usize;
            let uncomp_size = u32::from_le_bytes([bytes[i + 22], bytes[i + 23], bytes[i + 24], bytes[i + 25]]) as usize;
            let name_len = u16::from_le_bytes([bytes[i + 26], bytes[i + 27]]) as usize;
            let extra_len = u16::from_le_bytes([bytes[i + 28], bytes[i + 29]]) as usize;
            let name_start = i + 30;
            let name_end = name_start + name_len;
            if name_end > bytes.len() { break; }
            let name = std::str::from_utf8(&bytes[name_start..name_end]).unwrap_or("").to_lowercase();
            let data_start = name_end + extra_len;
            let data_end = data_start + comp_size;
            if data_end > bytes.len() { break; }
            let is_cov = name.ends_with("lcov.info") || name.ends_with("coverage.xml")
                || name.ends_with("coverage.out") || name.contains("cobertura");
            if is_cov {
                let data = &bytes[data_start..data_end];
                let text_opt: Option<String> = if method == 0 {
                    std::str::from_utf8(data).ok().map(String::from)
                } else if method == 8 {
                    // deflate via flate2
                    let mut d = flate2::read::DeflateDecoder::new(data);
                    let mut s = String::with_capacity(uncomp_size);
                    d.read_to_string(&mut s).ok().map(|_| s)
                } else { None };
                if let Some(text) = text_opt {
                    let fmt = if name.ends_with("lcov.info") { CoverageFormat::Lcov }
                        else if name.ends_with("coverage.xml") || name.contains("cobertura") { CoverageFormat::Cobertura }
                        else { CoverageFormat::GoCover };
                    return Some(RawCoverage { format: fmt, text });
                }
            }
            i = data_end;
        } else {
            i += 1;
        }
    }
    None
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CoverageConfig {
    pub lcov_path: Option<String>,
    pub cobertura_path: Option<String>,
    pub go_cover_path: Option<String>,
    pub exclude_patterns: Option<Vec<String>>,
}

pub fn line_str(h: Hit) -> &'static str {
    match h {
        Hit::Covered => "covered",
        Hit::Uncovered => "uncovered",
        Hit::NotInstrumented => "none",
    }
}
