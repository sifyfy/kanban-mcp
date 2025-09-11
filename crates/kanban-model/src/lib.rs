use anyhow::Result;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use time::{format_description::well_known::Rfc3339, OffsetDateTime};
use ulid::Ulid;

/// ULID utilities (uppercase, 26 chars)
pub fn new_ulid() -> String {
    Ulid::new().to_string().to_uppercase()
}

/// Column definitions loaded from `.kanban/columns.toml` (placeholder)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WatchToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hot_columns: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debounce_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_batch: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ColumnsToml {
    pub columns: Vec<String>,
    #[serde(default)]
    pub wip_limits: HashMap<String, usize>,
    #[serde(default)]
    pub watch: WatchToml,
    #[serde(default)]
    pub writer: WriterToml,
    #[serde(default)]
    pub render: RenderToml,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriterToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_rename_on_conflict: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rename_suffix: Option<String>,
}

/// Basic card front matter
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CardFrontMatter {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub labels: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assignees: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relates: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    // Optional fields for quick resume (LLM-friendly)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume_hint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_steps: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blockers: Option<Vec<String>>,
}

/// Card file wrapper (YAML front matter + Markdown body)
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CardFile {
    pub front_matter: CardFrontMatter,
    pub body: String,
}

impl CardFile {
    pub fn new_with_title(title: &str) -> Self {
        let now = OffsetDateTime::now_utc();
        let id = new_ulid();
        Self {
            front_matter: CardFrontMatter {
                id,
                title: title.to_string(),
                created_at: Some(now.format(&Rfc3339).unwrap_or_default()),
                ..Default::default()
            },
            body: String::new(),
        }
    }

    pub fn to_markdown(&self) -> Result<String> {
        let yaml = serde_yaml::to_string(&self.front_matter)?;
        Ok(format!("---\n{}---\n\n{}\n", yaml, self.body))
    }

    pub fn from_markdown(s: &str) -> Result<Self> {
        let re = Regex::new(r"(?s)^---\n(.*?)\n---\n\n?(.*)$").unwrap();
        if let Some(caps) = re.captures(s) {
            let fm: CardFrontMatter = serde_yaml::from_str(caps.get(1).unwrap().as_str())?;
            let body = caps
                .get(2)
                .map(|m| m.as_str())
                .unwrap_or_default()
                .to_string();
            Ok(Self {
                front_matter: fm,
                body,
            })
        } else {
            // No front matter; treat whole as body with empty FM
            Ok(Self {
                front_matter: CardFrontMatter::default(),
                body: s.to_string(),
            })
        }
    }
}

/// Filename helper: "<ULID>__<slug>.md"
pub fn filename_for(id: &str, title: &str) -> String {
    let mut slug = slug::slugify(title);
    if slug.is_empty() {
        slug = "card".to_string();
    }
    format!("{}__{}.md", id.to_uppercase(), slug)
}

impl fmt::Display for CardFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.to_markdown() {
            Ok(s) => write!(f, "{s}"),
            Err(_) => write!(f, "<invalid card markdown>"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ulid_is_uppercase_26() {
        let id = new_ulid();
        assert_eq!(id.len(), 26);
        assert!(id.chars().all(|c| !c.is_ascii_lowercase()));
    }

    #[test]
    fn fm_roundtrip() {
        let mut c = CardFile::new_with_title("Hello");
        c.body = "World".into();
        let s = c.to_markdown().unwrap();
        let c2 = CardFile::from_markdown(&s).unwrap();
        assert_eq!(c2.front_matter.title, "Hello");
        assert_eq!(c2.body.trim(), "World");
    }

    #[test]
    fn filename_pattern() {
        let name = filename_for("01ABCDEFGHJKLMNPQRSTVWXYZ", "Cool Title!");
        assert!(name.starts_with("01ABCDEFGHJKLMNPQRSTVWXYZ__cool-title"));
        assert!(name.ends_with(".md"));
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RenderToml {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debounce_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_parents: Option<Vec<String>>, // 複数親の進捗を出力
}

/// One journal entry (NDJSON per card)
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NoteEntry {
    pub ts: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
}
