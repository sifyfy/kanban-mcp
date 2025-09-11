use anyhow::Result;
use kanban_storage::Board;

fn count_files_in(dir: &std::path::Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    walkdir::WalkDir::new(dir)
        .min_depth(1)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .count()
}

pub fn render_simple_board(board: &Board) -> Result<String> {
    let base = board.root.join(".kanban");
    // columns from columns.toml or fallback
    let cols_cfg = {
        let p = base.join("columns.toml");
        if let Ok(t) = fs_err::read_to_string(p) {
            toml::from_str::<kanban_model::ColumnsToml>(&t).unwrap_or_default()
        } else {
            kanban_model::ColumnsToml::default()
        }
    };
    let mut cols = if cols_cfg.columns.is_empty() {
        vec!["backlog".into(), "doing".into(), "review".into()]
    } else {
        cols_cfg.columns.clone()
    };
    // ensure stable order and dedup
    cols.dedup();
    let mut out = String::new();
    out.push_str(
        "# Board

",
    );
    for c in &cols {
        let n = count_files_in(&base.join(c));
        out.push_str(&format!(
            "- {c}: {n}
"
        ));
    }
    let done = count_files_in(&base.join("done"));
    out.push_str(&format!("- done: {done}\n"));
    Ok(out)
}

pub fn render_board_with_template(board: &Board, template_text: &str) -> Result<String> {
    use serde_json::json;
    let base = board.root.join(".kanban");
    let cols_cfg = {
        let p = base.join("columns.toml");
        if let Ok(t) = fs_err::read_to_string(p) {
            toml::from_str::<kanban_model::ColumnsToml>(&t).unwrap_or_default()
        } else {
            kanban_model::ColumnsToml::default()
        }
    };
    let cols = if cols_cfg.columns.is_empty() {
        vec!["backlog".into(), "doing".into(), "review".into()]
    } else {
        cols_cfg.columns.clone()
    };
    let mut items = Vec::new();
    let mut non_done: usize = 0;
    for c in &cols {
        let n = count_files_in(&base.join(c));
        non_done += n;
        items.push(json!({"key": c, "count": n}));
    }
    let done = count_files_in(&base.join("done"));
    let total = non_done + done;
    let done_rate = if total > 0 {
        (done as f64) / (total as f64)
    } else {
        0.0
    };
    // Build progressParents (if configured)
    let mut progress_parents: Vec<serde_json::Value> = Vec::new();
    // Scan once for title map and by_parent
    use kanban_model::CardFile;
    let root = board.root.join(".kanban");
    let mut by_parent: std::collections::HashMap<String, Vec<CardFile>> =
        std::collections::HashMap::new();
    let mut title_map: std::collections::HashMap<String, (String, String)> =
        std::collections::HashMap::new();
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
                        title_map.insert(
                            card.front_matter.id.to_uppercase(),
                            (card.front_matter.title.clone(), String::new()),
                        );
                        if let Some(parent) = card.front_matter.parent.as_deref() {
                            by_parent
                                .entry(parent.to_uppercase())
                                .or_default()
                                .push(card);
                        }
                    }
                }
            }
        }
    }
    fn dfs(
        id: &str,
        by_parent: &std::collections::HashMap<String, Vec<CardFile>>,
    ) -> (u32, u32, u32, u32) {
        let mut done = 0;
        let mut total = 0;
        let mut done_size = 0;
        let mut total_size = 0;
        if let Some(ch) = by_parent.get(&id.to_uppercase()) {
            for c in ch {
                total += 1;
                if let Some(sz) = c.front_matter.size {
                    total_size += sz;
                }
                if c.front_matter.completed_at.is_some() {
                    done += 1;
                    if let Some(sz) = c.front_matter.size {
                        done_size += sz;
                    }
                }
                let (cd, ct, cds, cts) = dfs(&c.front_matter.id, by_parent);
                done += cd;
                total += ct;
                done_size += cds;
                total_size += cts;
            }
        }
        (done, total, done_size, total_size)
    }
    let parents_cfg: Vec<String> = if let Some(list) = cols_cfg.render.progress_parents.clone() {
        list
    } else if let Some(one) = cols_cfg.render.progress_parent.clone() {
        vec![one]
    } else {
        vec![]
    };
    for pid in parents_cfg {
        let up = pid.to_uppercase();
        let (title, _col) = title_map
            .get(&up)
            .cloned()
            .unwrap_or((String::new(), String::new()));
        let (d, t, ds, ts) = dfs(&up, &by_parent);
        let percent = if t > 0 {
            (d as f64) / (t as f64) * 100.0
        } else {
            0.0
        };
        let percent_size = if ts > 0 {
            (ds as f64) / (ts as f64) * 100.0
        } else {
            0.0
        };
        progress_parents.push(json!({
            "id": up,
            "title": title,
            "done": d,
            "total": t,
            "doneSize": ds,
            "totalSize": ts,
            "percent": (percent*10.0).round()/10.0,
            "percentSize": (percent_size*10.0).round()/10.0,
        }));
    }
    let ctx = json!({"columns": items, "done": done, "nonDone": non_done, "total": total, "doneRate": done_rate});
    let hb = handlebars::Handlebars::new();
    // enrich context
    let mut ctx_obj = ctx.as_object().cloned().unwrap_or_default();
    ctx_obj.insert("progressParents".into(), json!(progress_parents));
    Ok(hb.render_template(template_text, &serde_json::Value::Object(ctx_obj))?)
}

pub fn render_parent_progress(board: &Board, parent_id: &str) -> Result<String> {
    // minimal rollup: count children (direct + transitive) and size sums
    use kanban_model::CardFile;
    let root = board.root.join(".kanban");
    let mut by_parent: std::collections::HashMap<String, Vec<CardFile>> =
        std::collections::HashMap::new();
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
                        if let Some(parent) = card.front_matter.parent.as_deref() {
                            by_parent
                                .entry(parent.to_uppercase())
                                .or_default()
                                .push(card);
                        }
                    }
                }
            }
        }
    }
    fn dfs(
        id: &str,
        by_parent: &std::collections::HashMap<String, Vec<CardFile>>,
    ) -> (u32, u32, u32, u32) {
        let mut done = 0;
        let mut total = 0;
        let mut done_size = 0;
        let mut total_size = 0;
        if let Some(ch) = by_parent.get(&id.to_uppercase()) {
            for c in ch {
                total += 1;
                if let Some(sz) = c.front_matter.size {
                    total_size += sz;
                }
                if c.front_matter.completed_at.is_some() {
                    done += 1;
                    if let Some(sz) = c.front_matter.size {
                        done_size += sz;
                    }
                }
                let (cd, ct, cds, cts) = dfs(&c.front_matter.id, by_parent);
                done += cd;
                total += ct;
                done_size += cds;
                total_size += cts;
            }
        }
        (done, total, done_size, total_size)
    }
    let (done, total, done_size, total_size) = dfs(&parent_id.to_uppercase(), &by_parent);
    let pct = if total > 0 {
        (done as f64) / (total as f64) * 100.0
    } else {
        0.0
    };
    let pct_s = if total_size > 0 {
        (done_size as f64) / (total_size as f64) * 100.0
    } else {
        0.0
    };
    Ok(format!(
        "progress: {done}/{total} ({pct:.1}%) size: {done_size}/{total_size} ({pct_s:.1}%)"
    ))
}
