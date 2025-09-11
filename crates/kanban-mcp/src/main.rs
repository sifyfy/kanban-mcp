use clap::{Parser, Subcommand};
use kanban_mcp::{JsonRpcResponse, Server};
use serde_json::Value;
use std::io::{self, BufRead, Write};
use tracing::{error, info, Level};

#[derive(Parser, Debug)]
#[command(name = "kanban", version, about = "File-based Kanban MCP + CLI")]
struct Cli {
    /// Path to board root (directory containing .kanban/)
    #[arg(long, global = true, default_value = ".")]
    board: String,

    /// Log level (trace|debug|info|warn|error)
    #[arg(long, global = true, default_value = "info")]
    log_level: String,


    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start MCP server over stdio
    Mcp {},
    /// Lint board (relations/parent_done/wip)
    Lint {
        /// Output JSON array instead of human text
        #[arg(long)]
        json: bool,
        /// Fail on: error|warn (error by default)
        #[arg(long, default_value = "error")]
        fail_on: String,
    },
    /// Reindex cards/relations ndjson
    Reindex {
        #[arg(long)]
        cards_only: bool,
        #[arg(long)]
        relations_only: bool,
    },
    /// Compact done partitions / cleanup (safe subset)
    Compact {
        /// Show actions without applying
        #[arg(long)]
        dry_run: bool,
        /// Remove empty dirs under .kanban after moves
        #[arg(long, default_value_t = true)]
        remove_empty_dirs: bool,
    },
    /// Notes (journal) helpers
    NotesAppend {
        /// Card ULID
        #[arg(long)]
        card_id: String,
        /// Note text (short paragraphs recommended)
        #[arg(long)]
        text: String,
        /// Note type: worklog|resume|decision (default worklog)
        #[arg(long, default_value = "worklog")]
        r#type: String,
        /// Optional comma-separated tags
        #[arg(long)]
        tags: Option<String>,
        /// Optional author
        #[arg(long)]
        author: Option<String>,
        /// Read text from file (overrides --text)
        #[arg(long, value_name = "PATH")]
        from_file: Option<String>,
    },
    NotesList {
        /// Card ULID
        #[arg(long)]
        card_id: String,
        /// Return all notes
        #[arg(long, default_value_t = false)]
        all: bool,
        /// Limit (when --all is false)
        #[arg(long, default_value_t = 3)]
        limit: usize,
        /// Output JSON
        #[arg(long, default_value_t = false)]
        json: bool,
        /// Filter notes since RFC3339 timestamp (e.g., 2025-09-05T12:00:00Z)
        #[arg(long, value_name = "RFC3339")]
        since: Option<String>,
        /// Output format: plain|markdown (default plain)
        #[arg(long, value_name = "FMT")]
        format: Option<String>,
        /// Include header (card id/title) for markdown format
        #[arg(long, default_value_t = false)]
        with_header: bool,
        /// Include kanban:// link in header (markdown format)
        #[arg(long, default_value_t = false)]
        link: bool,
    },
    /// Update front-matter quick resume fields
    UpdateFm {
        /// Card ULID
        #[arg(long)]
        card_id: String,
        /// Resume hint (short)
        #[arg(long)]
        resume_hint: Option<String>,
        /// Next steps (repeatable)
        #[arg(long, value_name = "STEP")]
        next: Vec<String>,
        /// Blockers (repeatable)
        #[arg(long, value_name = "BLOCKER")]
        blocker: Vec<String>,
    },
}

fn init_logging(level: &str) {
    let max = match level.to_ascii_lowercase().as_str() {
        "trace" => Level::TRACE,
        "debug" => Level::DEBUG,
        "warn" => Level::WARN,
        "error" => Level::ERROR,
        _ => Level::INFO,
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(max)
        .with_target(false)
        .with_writer(std::io::stderr)
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}

fn run_mcp_stdio() {
    info!("kanban mcp (stdio) started");
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    tracing::info!(target: "kanban_mcp", "stdio loop starting");
    for line in stdin.lock().lines() {
        match line {
            Ok(l) => {
                let l = l.trim();
                if l.is_empty() {
                    continue;
                }
                tracing::debug!(target: "kanban_mcp", "[REQ] {}", l);
                let req: Result<Value, _> = serde_json::from_str(l);
                let (maybe_id, resp_val) = match req {
                    Ok(v) => {
                        let maybe_id = v.get("id").cloned();
                        match Server::handle_value(v) {
                            Ok(r) => (maybe_id, r),
                            Err(e) => (
                                maybe_id,
                                serde_json::to_value(JsonRpcResponse::error(
                                    None,
                                    -32000,
                                    &format!("internal: {e}"),
                                    None,
                                ))
                                .unwrap(),
                            ),
                        }
                    }
                    Err(e) => (None, serde_json::to_value(JsonRpcResponse::error(
                        None,
                        -32700,
                        &format!("parse error: {e}"),
                        None,
                    ))
                    .unwrap()),
                };
                // Do not respond to notifications (no id per JSON-RPC spec)
                let should_reply = !matches!(maybe_id, None);
                if should_reply {
                    let s = serde_json::to_string(&resp_val).unwrap();
                    writeln!(stdout, "{s}").ok();
                    tracing::debug!(target: "kanban_mcp", "[RSP] {}", s);
                    stdout.flush().ok();
                }
            }
            Err(e) => {
                error!("stdin read error: {}", e);
                break;
            }
        }
    }
}

fn main() {
    let cli = Cli::parse();
    init_logging(&cli.log_level);
    info!("logging initialized (level={})", cli.log_level);

    match cli.command {
        Commands::Mcp {} => run_mcp_stdio(),
        Commands::Lint { json, fail_on } => {
            use kanban_lint::{lint_parent_done, lint_relations, lint_wip};
            use kanban_model::ColumnsToml;
            use kanban_storage::Board;
            let board = Board::new(&cli.board);

            let mut issues: Vec<String> = vec![];
            if let Ok(toml_text) =
                fs_err::read_to_string(board.root.join(".kanban").join("columns.toml"))
            {
                if let Ok(cfg) = toml::from_str::<ColumnsToml>(&toml_text) {
                    if let Ok(mut w) = lint_wip(&board, &cfg) {
                        issues.append(&mut w);
                    }
                }
            }
            if let Ok(mut r) = lint_relations(&board) {
                issues.append(&mut r);
            }
            if let Ok(mut p) = lint_parent_done(&board) {
                issues.append(&mut p);
            }

            fn classify(msg: &str) -> &'static str {
                let m = msg.to_ascii_lowercase();
                if m.contains("missing id") || m.contains("missing title") {
                    return "error";
                }
                if m.contains("dangling ") || m.contains("cycle") {
                    return "error";
                }
                if m.contains("self ") {
                    return "warn";
                }
                if m.contains("wip exceeded") {
                    return "warn";
                }
                if m.contains("parent done but child not complete") {
                    return "warn";
                }
                "warn"
            }

            let classified: Vec<serde_json::Value> = issues
                .iter()
                .map(|m| {
                    serde_json::json!({
                        "severity": classify(m),
                        "message": m,
                    })
                })
                .collect();
            let error_cnt = classified
                .iter()
                .filter(|v| v.get("severity").and_then(|s| s.as_str()) == Some("error"))
                .count();

            if json {
                println!("{}", serde_json::to_string_pretty(&classified).unwrap());
            } else {
                for v in &classified {
                    let sev = v.get("severity").and_then(|s| s.as_str()).unwrap_or("warn");
                    let msg = v.get("message").and_then(|s| s.as_str()).unwrap_or("");
                    println!("{} {}", sev.to_uppercase(), msg);
                }
                if classified.is_empty() { println!("OK no issues"); }
            }

            let fail_on = fail_on.to_ascii_lowercase();
            let exit_fail = if fail_on == "warn" {
                !classified.is_empty()
            } else {
                error_cnt > 0
            };
            std::process::exit(if exit_fail { 1 } else { 0 });
        }
        Commands::Reindex {
            cards_only,
            relations_only,
        } => {
            use kanban_storage::Board;
            let board = Board::new(&cli.board);
            let t0 = std::time::Instant::now();
            let mut errors: Vec<String> = vec![];
            if !relations_only {
                if let Err(e) = board.reindex_cards() {
                    errors.push(format!("cards: {e}"));
                }
            }
            if !cards_only {
                if let Err(e) = board.reindex_relations() {
                    errors.push(format!("relations: {e}"));
                }
            }
            let dur = t0.elapsed().as_millis();
                println!(
                    "{}",
                    serde_json::json!({"duration_ms": dur, "errors": errors})
                );
            std::process::exit(if errors.is_empty() { 0 } else { 1 });
        }
        Commands::Compact {
            dry_run,
            remove_empty_dirs,
        } => {
            use kanban_model::CardFile;
            use kanban_storage::Board;
            let board = Board::new(&cli.board);
            let base = board.root.join(".kanban");
            let done_dir = base.join("done");
            let mut moves: Vec<(String, String)> = vec![];
            if done_dir.exists() {
                for e in walkdir::WalkDir::new(&done_dir)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if e.file_type().is_file() {
                        let p = e.path().to_path_buf();
                        if !p
                            .extension()
                            .and_then(|s| s.to_str())
                            .map(|s| s.eq_ignore_ascii_case("md"))
                            .unwrap_or(false)
                        {
                            continue;
                        }
                        // is already under done/YYYY/MM/?
                        let rel = p.strip_prefix(&done_dir).unwrap();
                        let depth = rel.components().count();
                        let needs_move = depth < 3; // not under YYYY/MM
                        if needs_move {
                            // determine year/month from completed_at or mtime
                            let (year, month) = if let Ok(text) = fs_err::read_to_string(&p) {
                                if let Ok(card) = CardFile::from_markdown(&text) {
                                    if let Some(ca) = card.front_matter.completed_at.as_deref() {
                                        if ca.len() >= 7 {
                                            if let (Ok(y), Ok(m)) =
                                                (ca[0..4].parse::<i32>(), ca[5..7].parse::<u8>())
                                            {
                                                (y, m)
                                            } else {
                                                (1970, 1)
                                            }
                                        } else {
                                            (1970, 1)
                                        }
                                    } else {
                                        (1970, 1)
                                    }
                                } else {
                                    (1970, 1)
                                }
                            } else {
                                (1970, 1)
                            };
                            let y = format!("{year:04}");
                            let m = format!("{month:02}");
                            let fname = p.file_name().unwrap().to_string_lossy().to_string();
                            let dest = done_dir.join(&y).join(&m).join(&fname);
                            moves.push((
                                p.to_string_lossy().to_string(),
                                dest.to_string_lossy().to_string(),
                            ));
                        }
                    }
                }
            }
            if dry_run {
                println!(
                    "{}",
                    serde_json::json!({"moves": moves, "remove_empty_dirs": remove_empty_dirs})
                );
                return;
            }
            // apply moves
            for (from, to) in &moves {
                let fromp = std::path::Path::new(from);
                let top = std::path::Path::new(to);
                if let Some(parent) = top.parent() {
                    let _ = fs_err::create_dir_all(parent);
                }
                let _ = fs_err::rename(fromp, top);
            }
            // remove empty dirs if requested
            if remove_empty_dirs && base.exists() {
                // Walk bottom-up
                let mut dirs: Vec<std::path::PathBuf> = vec![];
                for e in walkdir::WalkDir::new(&base)
                    .into_iter()
                    .filter_map(|e| e.ok())
                {
                    if e.file_type().is_dir() {
                        dirs.push(e.path().to_path_buf());
                    }
                }
                dirs.sort_by_key(|b| std::cmp::Reverse(b.components().count()));
                for d in dirs {
                    if d == base {
                        continue;
                    }
                    if let Ok(mut it) = std::fs::read_dir(&d) {
                        if it.next().is_none() {
                            let _ = fs_err::remove_dir(&d);
                        }
                    }
                }
            }
            println!("{}", serde_json::json!({"moved": moves.len(), "ok": true}));
        }
        Commands::NotesAppend {
            card_id,
            text,
            r#type,
            tags,
            author,
            from_file,
        } => {
            use kanban_model::NoteEntry;
            use kanban_storage::Board;
            let board = Board::new(&cli.board);
            let text = if let Some(path) = from_file.as_ref() {
                match fs_err::read_to_string(path) {
                    Ok(t) => t,
                    Err(e) => {
                eprintln!("failed to read --from-file: {e}");
                        std::process::exit(1);
                    }
                }
            } else {
                text
            };
            let ts = time::OffsetDateTime::now_utc()
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default();
            let tags_vec = tags.map(|s| {
                s.split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect::<Vec<_>>()
            });
            let entry = NoteEntry {
                ts: ts.clone(),
                type_: r#type,
                text,
                tags: tags_vec,
                author,
            };
            if let Err(e) = board.append_note(&card_id, &entry) {
                eprintln!("append failed: {e}");
                std::process::exit(1);
            }
            println!("{}", serde_json::json!({"appended": true, "ts": ts}));
        }
        Commands::NotesList {
            card_id,
            all,
            limit,
            json,
            since,
            format,
            with_header,
            link,
        } => {
            use kanban_storage::Board;
            let board = Board::new(&cli.board);
            match board.list_notes_advanced(&card_id, Some(limit), all, since.as_deref()) {
                Ok(items) => {
                    if json {
                        println!("{}", serde_json::to_string_pretty(&items).unwrap());
                    } else if matches!(format.as_deref(), Some("markdown")) {
                        if with_header {
                            let title = board
                                .read_card(&card_id)
                                .ok()
                                .map(|c| c.front_matter.title)
                                .unwrap_or_default();
                            if let Some(ref s) = since {
                        println!(
                            "### Notes (latest {}) for [{}] {} (since {})",
                            items.len(),
                            card_id,
                            title,
                            s
                        );
                            } else {
                                println!(
                                    "### Notes (latest {}) for [{}] {}",
                                    items.len(),
                                    card_id,
                                    title
                                );
                            }
                            if link {
                                println!("<kanban://{}/cards/{}>", cli.board, card_id);
                            }
                        }
                        for it in items {
                            let tags = it.tags.unwrap_or_default();
                            let tags_md = if tags.is_empty() {
                                String::new()
                            } else {
                                format!(" [{}]", tags.join(","))
                            };
                            let author = it.author.unwrap_or_default();
                            println!("- [{}] {}{} {}", it.ts, it.type_, tags_md, author);
                            println!();
                            println!("  {}", it.text);
                            println!();
                        }
                    } else {
                        for it in items {
                            let tags = it.tags.unwrap_or_default().join(",");
                            let author = it.author.unwrap_or_default();
                            println!(
                                "- [{}] {} {} {}",
                                it.ts,
                                it.type_,
                                author,
                                if tags.is_empty() {
                                    "".into()
                                } else {
                                    format!("[{tags}]")
                                }
                            );
                            println!("  {}", it.text);
                        }
                    }
                }
                Err(e) => {
                    eprintln!("list failed: {e}");
                    std::process::exit(1);
                }
            }
        }
        Commands::UpdateFm {
            card_id,
            resume_hint,
            next,
            blocker,
        } => {
            use serde_json::json;
            let mut fm = serde_json::Map::new();
            if let Some(h) = resume_hint.as_ref() {
                fm.insert("resume_hint".into(), json!(h));
            }
            if !next.is_empty() {
                fm.insert("next_steps".into(), json!(next));
            }
            if !blocker.is_empty() {
                fm.insert("blockers".into(), json!(blocker));
            }
            if fm.is_empty() {
                eprintln!("no fields to update");
                std::process::exit(1);
            }
            let req = json!({
                "jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"kanban/update","arguments":{"board": &cli.board, "cardId": card_id, "patch": {"fm": serde_json::Value::Object(fm)} }}
            });
            match kanban_mcp::Server::handle_value(req) {
                Ok(v) => println!("{v}"),
                Err(e) => {
                    eprintln!("update failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}
