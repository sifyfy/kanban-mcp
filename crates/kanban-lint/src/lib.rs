use anyhow::Result;
use kanban_model::CardFile;
use kanban_storage::Board;
use std::collections::{HashMap, HashSet};

pub fn lint_required_fields(card: &CardFile) -> Result<Vec<String>> {
    let mut warnings = vec![];
    if card.front_matter.id.is_empty() {
        warnings.push("missing id".into());
    }
    if card.front_matter.title.is_empty() {
        warnings.push("missing title".into());
    }
    Ok(warnings)
}

pub fn lint_wip(root: &Board, columns_toml: &kanban_model::ColumnsToml) -> Result<Vec<String>> {
    if columns_toml.wip_limits.is_empty() {
        return Ok(vec![]);
    }
    use std::collections::HashMap;
    let mut counts: HashMap<String, usize> = HashMap::new();
    let base = root.root.join(".kanban");
    for col in columns_toml.wip_limits.keys() {
        let dir = base.join(col);
        if dir.exists() {
            let mut c = 0usize;
            for e in walkdir::WalkDir::new(&dir)
                .min_depth(1)
                .max_depth(1)
                .into_iter()
                .flatten()
            {
                if e.file_type().is_file() {
                    c += 1;
                }
            }
            counts.insert(col.clone(), c);
        } else {
            counts.insert(col.clone(), 0);
        }
    }
    let mut issues = vec![];
    for (col, lim) in &columns_toml.wip_limits {
        if let Some(cnt) = counts.get(col) {
            if *cnt > *lim {
                issues.push(format!("wip exceeded: {col} limit {lim} actual {cnt}"));
            }
        }
    }
    Ok(issues)
}

fn scan_cards(root: &Board) -> Result<Vec<(std::path::PathBuf, CardFile)>> {
    let base = root.root.join(".kanban");
    let mut out = vec![];
    if base.exists() {
        for e in walkdir::WalkDir::new(&base)
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
                        out.push((p.to_path_buf(), card));
                    }
                }
            }
        }
    }
    Ok(out)
}

pub fn lint_relations(root: &Board) -> Result<Vec<String>> {
    let cards = scan_cards(root)?;
    let mut ids: HashSet<String> = HashSet::new();
    let mut parent_of: HashMap<String, String> = HashMap::new();
    for (_p, c) in &cards {
        ids.insert(c.front_matter.id.to_uppercase());
        if let Some(p) = c.front_matter.parent.as_deref() {
            parent_of.insert(c.front_matter.id.to_uppercase(), p.to_uppercase());
        }
    }
    let mut issues = vec![];
    for (_p, c) in &cards {
        let idu = c.front_matter.id.to_uppercase();
        if let Some(p) = c.front_matter.parent.as_deref() {
            let pu = p.to_uppercase();
            if !ids.contains(&pu) {
                issues.push(format!("dangling parent: {idu} -> {pu}"));
            }
        }
        if let Some(ds) = c.front_matter.depends_on.as_ref() {
            for d in ds {
                let du = d.to_uppercase();
                if !ids.contains(&du) {
                    issues.push(format!("dangling depends: {idu} -> {du}"));
                }
                if du == idu {
                    issues.push(format!("self depends: {idu}"));
                }
            }
        }
        if let Some(rs) = c.front_matter.relates.as_ref() {
            for r in rs {
                let ru = r.to_uppercase();
                if !ids.contains(&ru) {
                    issues.push(format!("dangling relates: {idu} <-> {ru}"));
                }
                if ru == idu {
                    issues.push(format!("self relates: {idu}"));
                }
            }
        }
    }
    for id in ids.iter() {
        let mut seen: HashSet<String> = HashSet::new();
        let mut cur = id.clone();
        let mut depth = 0;
        while let Some(p) = parent_of.get(&cur) {
            if !seen.insert(cur.clone()) {
                issues.push(format!("parent cycle detected at {id}"));
                break;
            }
            cur = p.clone();
            depth += 1;
            if depth > 1000 {
                break;
            }
        }
    }
    Ok(issues)
}

pub fn lint_parent_done(root: &Board) -> Result<Vec<String>> {
    let cards = scan_cards(root)?;
    let mut by_parent: HashMap<String, Vec<CardFile>> = HashMap::new();
    let mut by_id: HashMap<String, CardFile> = HashMap::new();
    for (_p, c) in cards.into_iter() {
        let idu = c.front_matter.id.to_uppercase();
        if let Some(p) = c.front_matter.parent.as_deref() {
            by_parent
                .entry(p.to_uppercase())
                .or_default()
                .push(c.clone());
        }
        by_id.insert(idu, c);
    }
    let mut issues = vec![];
    for (pid, children) in by_parent.into_iter() {
        if let Some(pcard) = by_id.get(&pid) {
            let parent_done = pcard.front_matter.completed_at.is_some();
            if parent_done {
                for ch in children.iter() {
                    if ch.front_matter.completed_at.is_none() {
                        issues.push(format!(
                            "parent done but child not complete: {} -> {}",
                            pid, ch.front_matter.id
                        ));
                    }
                }
            }
        }
    }
    Ok(issues)
}
