use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

use kanban_model::NoteEntry;
use kanban_model::{filename_for, CardFile};
use serde_json::json;
use std::io::Write;

#[derive(Debug, Clone)]
pub struct Board {
    pub root: PathBuf,
}

impl Board {
    pub fn new(root: impl AsRef<Path>) -> Self {
        Self {
            root: root.as_ref().to_path_buf(),
        }
    }

    pub fn append_note(&self, id: &str, entry: &NoteEntry) -> Result<()> {
        let base = self.root.join(".kanban").join("notes");
        fs_err::create_dir_all(&base)?;
        let path = base.join(format!("{}.ndjson", id.to_uppercase()));
        let mut f = fs_err::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        let line = serde_json::to_string(entry)?;
        writeln!(f, "{line}")?;
        Ok(())
    }

    pub fn list_notes(&self, id: &str, limit: Option<usize>, all: bool) -> Result<Vec<NoteEntry>> {
        self.list_notes_advanced(id, limit, all, None)
    }

    pub fn list_notes_advanced(
        &self,
        id: &str,
        limit: Option<usize>,
        all: bool,
        since: Option<&str>,
    ) -> Result<Vec<NoteEntry>> {
        let path = self
            .root
            .join(".kanban")
            .join("notes")
            .join(format!("{}.ndjson", id.to_uppercase()));
        if !path.exists() {
            return Ok(vec![]);
        }
        let text = fs_err::read_to_string(&path)?;
        let mut items: Vec<NoteEntry> = vec![];
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<NoteEntry>(line) {
                if let Some(since_s) = since {
                    // Best-effort string compare (our timestamps are RFC3339 UTC by default)
                    if v.ts.as_str() < since_s {
                        continue;
                    }
                }
                items.push(v);
            }
        }
        // Newest last in file; return reverse (newest first)
        items.reverse();
        if all {
            return Ok(items);
        }
        let n = limit.unwrap_or(3);
        Ok(items.into_iter().take(n).collect())
    }

    pub fn new_card(
        &self,
        title: &str,
        lane: Option<String>,
        priority: Option<String>,
        size: Option<u32>,
        column: &str,
    ) -> Result<String> {
        let mut card = CardFile::new_with_title(title);
        card.front_matter.lane = lane;
        card.front_matter.priority = priority;
        card.front_matter.size = size;

        let id = card.front_matter.id.clone();
        let filename = filename_for(&id, title);
        let dir = self.root.join(".kanban").join(column);
        fs_err::create_dir_all(&dir)?;
        let path = dir.join(filename);
        fs_err::write(&path, card.to_markdown()?)?;
        // index upsert
        self.upsert_card_index(&card, column)?;
        Ok(id)
    }

    pub fn read_card_text(&self, id: &str) -> Result<String> {
        let (path, _fm) = self.find_path_by_id(id)?;
        Ok(fs_err::read_to_string(path)?)
    }

    pub fn read_card(&self, id: &str) -> Result<CardFile> {
        let text = self.read_card_text(id)?;
        CardFile::from_markdown(&text)
    }

    pub fn move_card(&self, id: &str, to_column: &str) -> Result<()> {
        let (path, fm) = self.find_path_by_id(id)?;
        let filename = filename_for(&fm.id, &fm.title);
        let dest_dir = self.root.join(".kanban").join(to_column);
        fs_err::create_dir_all(&dest_dir)?;
        let dest = dest_dir.join(filename);
        fs_err::rename(path, dest)?;
        // index upsert with new column
        let card = self.read_card(id)?;
        self.upsert_card_index(&card, to_column)?;
        Ok(())
    }

    pub fn done_card(&self, id: &str) -> Result<()> {
        let (path, mut card) = {
            let (p, _fm) = self.find_path_by_id(id)?;
            let text = fs_err::read_to_string(&p)?;
            (p, CardFile::from_markdown(&text)?)
        };
        card.front_matter.completed_at = Some(
            OffsetDateTime::now_utc()
                .format(&Rfc3339)
                .unwrap_or_default(),
        );
        fs_err::write(&path, card.to_markdown()?)?;
        let now = OffsetDateTime::now_utc();
        let year = now.year();
        let month: u8 = now.month().into();
        let dest_dir = self
            .root
            .join(".kanban")
            .join("done")
            .join(format!("{year:04}"))
            .join(format!("{month:02}"));
        fs_err::create_dir_all(&dest_dir)?;
        let filename = filename_for(&card.front_matter.id, &card.front_matter.title);
        let dest = dest_dir.join(filename);
        fs_err::rename(path, dest)?;
        // index upsert with new column
        let card = self.read_card(id)?;
        self.upsert_card_index(&card, "done")?;
        Ok(())
    }

    pub fn list_ids(&self, column: &str) -> Result<Vec<String>> {
        let dir = self.root.join(".kanban").join(column);
        let mut ids = vec![];
        if dir.exists() {
            for entry in walkdir::WalkDir::new(dir).min_depth(1).max_depth(1) {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some((id, _rest)) = name.split_once("__") {
                    ids.push(id.to_string());
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    pub fn reindex_cards(&self) -> Result<()> {
        use serde_json::json;
        let root = self.root.join(".kanban");
        fs_err::create_dir_all(&root)?;
        let idx = root.join("cards.ndjson");
        let mut out = String::new();
        if root.exists() {
            for e in walkdir::WalkDir::new(&root)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if e.file_type().is_file() {
                    let p = e.path();
                    if !p
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.eq_ignore_ascii_case("md"))
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    let rel = p.strip_prefix(&root).unwrap();
                    let mut comps = rel.components();
                    let first = comps
                        .next()
                        .and_then(|c| c.as_os_str().to_str())
                        .unwrap_or("");
                    let column = if first.eq_ignore_ascii_case("done") {
                        "done".to_string()
                    } else {
                        first.to_string()
                    };
                    let text = match fs_err::read_to_string(p) {
                        Ok(t) => t,
                        Err(_) => continue,
                    };
                    if let Ok(card) = CardFile::from_markdown(&text) {
                        let v = json!({
                            "id": card.front_matter.id,
                            "title": card.front_matter.title,
                            "column": column,
                            "lane": card.front_matter.lane,
                            "priority": card.front_matter.priority,
                            "labels": card.front_matter.labels,
                            "assignees": card.front_matter.assignees,
                            "completed_at": card.front_matter.completed_at,
                        });
                        out.push_str(&serde_json::to_string(&v)?);
                        out.push('\n');
                    }
                }
            }
        }
        fs_err::write(idx, out)?;
        Ok(())
    }

    pub fn reindex_relations(&self) -> Result<()> {
        use serde_json::json;
        let root = self.root.join(".kanban");
        fs_err::create_dir_all(&root)?;
        let idx = root.join("relations.ndjson");
        let mut out = String::new();
        let mut ids = std::collections::HashSet::new();
        let mut cards: Vec<CardFile> = vec![];
        if root.exists() {
            for e in walkdir::WalkDir::new(&root)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                if e.file_type().is_file() {
                    let p = e.path();
                    if !p
                        .extension()
                        .and_then(|s| s.to_str())
                        .map(|s| s.eq_ignore_ascii_case("md"))
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    if let Ok(text) = fs_err::read_to_string(p) {
                        if let Ok(card) = CardFile::from_markdown(&text) {
                            ids.insert(card.front_matter.id.to_uppercase());
                            cards.push(card);
                        }
                    }
                }
            }
        }
        for c in cards {
            let idu = c.front_matter.id.to_uppercase();
            if let Some(p) = c.front_matter.parent.as_deref() {
                let v = json!({"type":"parent","from": idu, "to": p.to_uppercase()});
                out.push_str(&serde_json::to_string(&v)?);
                out.push('\n');
            }
            if let Some(ds) = c.front_matter.depends_on.as_ref() {
                for d in ds {
                    let v = json!({"type":"depends","from": idu, "to": d.to_uppercase()});
                    out.push_str(&serde_json::to_string(&v)?);
                    out.push('\n');
                }
            }
            if let Some(rs) = c.front_matter.relates.as_ref() {
                for r in rs {
                    let v = json!({"type":"relates","from": idu, "to": r.to_uppercase()});
                    out.push_str(&serde_json::to_string(&v)?);
                    out.push('\n');
                }
            }
        }
        fs_err::write(idx, out)?;
        Ok(())
    }

    pub fn compact_dirs(&self) -> Result<()> {
        // No-op minimal implementation
        Ok(())
    }

    pub fn set_parent(&self, _child: &str, _parent: Option<&str>) -> Result<()> {
        bail!("unimplemented: set_parent")
    }
    pub fn add_depends(&self, _from: &str, _to: &str) -> Result<()> {
        bail!("unimplemented: add_depends")
    }
    pub fn remove_depends(&self, _from: &str, _to: &str) -> Result<()> {
        bail!("unimplemented: remove_depends")
    }
    pub fn add_relates(&self, _a: &str, _b: &str) -> Result<()> {
        bail!("unimplemented: add_relates")
    }
    pub fn remove_relates(&self, _a: &str, _b: &str) -> Result<()> {
        bail!("unimplemented: remove_relates")
    }

    pub fn split_new_parent_with_children(
        &self,
        _parent_title: &str,
        _lane: Option<String>,
        _priority: Option<String>,
        _psize: Option<u32>,
        _column: &str,
        _children_titles: &[String],
    ) -> Result<String> {
        bail!("unimplemented: split_new_parent_with_children")
    }

    pub fn rollup_count_size(&self, _root_id: &str) -> Result<(u32, u32, u32, u32)> {
        bail!("unimplemented: rollup_count_size")
    }

    fn find_path_by_id(&self, id: &str) -> Result<(PathBuf, kanban_model::CardFrontMatter)> {
        let root = self.root.join(".kanban");
        if !root.exists() {
            bail!(".kanban not found: {}", root.display());
        }
        for entry in walkdir::WalkDir::new(&root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some((fid, _)) = name.split_once("__") {
                    if fid.eq_ignore_ascii_case(id) {
                        let text = fs_err::read_to_string(entry.path())?;
                        let cf = CardFile::from_markdown(&text)?;
                        return Ok((entry.path().to_path_buf(), cf.front_matter));
                    }
                }
            }
        }
        bail!("card not found: {}", id)
    }
}

#[cfg(test)]
mod tests_notes_storage {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_and_list_basic_and_limit() {
        let tmp = tempdir().unwrap();
        let b = Board::new(tmp.path());
        // prepare an id
        let id = "01TESTNOTE0000000000000000";
        // append 4 entries
        for i in 0..4u8 {
            let e = kanban_model::NoteEntry {
                ts: time::OffsetDateTime::now_utc()
                    .format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default(),
                type_: "worklog".into(),
                text: format!("entry-{i}"),
                tags: None,
                author: None,
            };
            b.append_note(id, &e).unwrap();
        }
        // default latest (limit None, all=false) uses 3
        let v = b.list_notes(id, None, false).unwrap();
        assert_eq!(v.len(), 3);
        // all=true returns >=4
        let v2 = b.list_notes(id, Some(10), true).unwrap();
        assert!(v2.len() >= 4);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ListFilter {
    pub columns: Option<Vec<String>>,
    pub lane: Option<String>,
    pub priority: Option<String>,
    pub label: Option<String>,
    pub assignee: Option<String>,
    pub query: Option<String>,
    pub include_done: bool,
    pub offset: Option<usize>,
    pub limit: Option<usize>,
}

impl Board {
    pub fn list_cards_filtered(&self, _filter: &ListFilter) -> Result<Vec<String>> {
        // Minimal stub
        Ok(vec![])
    }

    pub fn upsert_card_index(
        &self,
        card: &kanban_model::CardFile,
        column: &str,
    ) -> anyhow::Result<()> {
        let base = self.root.join(".kanban");
        fs_err::create_dir_all(&base)?;
        let idx = base.join("cards.ndjson");
        let mut lines: Vec<String> = Vec::new();
        if idx.exists() {
            let text = fs_err::read_to_string(&idx)?;
            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    if v.get("id").and_then(|x| x.as_str()) == Some(card.front_matter.id.as_str()) {
                        continue;
                    }
                }
                lines.push(line.to_string());
            }
        }
        let v = json!({
            "id": card.front_matter.id,
            "title": card.front_matter.title,
            "column": column,
            "lane": card.front_matter.lane,
            "priority": card.front_matter.priority,
            "labels": card.front_matter.labels,
            "assignees": card.front_matter.assignees,
            "completed_at": card.front_matter.completed_at,
        });
        lines.push(serde_json::to_string(&v)?);
        let mut tmp = tempfile::NamedTempFile::new_in(&base)?;
        for l in lines {
            writeln!(tmp, "{l}")?;
        }
        tmp.persist(idx)?;
        Ok(())
    }
}
