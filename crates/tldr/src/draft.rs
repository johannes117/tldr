use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Draft {
    pub pr: u64,
    pub body: String,
    pub verdict: Option<String>, // "approve" | "request_changes" | "comment"
    pub comments: Vec<DraftComment>,
    pub file_state: HashMap<String, FileState>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftComment {
    pub id: String,
    pub path: String,
    pub line: u32,
    pub side: String, // "RIGHT" | "LEFT"
    pub body: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct FileState {
    pub viewed: bool,
    pub collapsed: bool,
}

pub fn load(path: &Path, pr: u64) -> Result<Draft> {
    if path.exists() {
        let s = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&s).unwrap_or_default())
    } else {
        Ok(Draft { pr, updated_at: Utc::now().to_rfc3339(), ..Default::default() })
    }
}

pub fn save(path: &Path, draft: &Draft) -> Result<()> {
    if let Some(p) = path.parent() { std::fs::create_dir_all(p).ok(); }
    let mut d = draft.clone();
    d.updated_at = Utc::now().to_rfc3339();
    std::fs::write(path, serde_json::to_string_pretty(&d)?)?;
    Ok(())
}

pub fn new_comment_id() -> String { Uuid::new_v4().to_string() }
