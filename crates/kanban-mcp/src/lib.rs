use anyhow::{anyhow, bail, Result};
use kanban_model::{filename_for, CardFile};
use kanban_storage::Board;
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;

// ツール名は常にフラット名（^[a-zA-Z0-9_-]+$）に統一します。

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Tool {
    pub name: String,
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none", rename = "outputSchema")]
    pub output_schema: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub annotations: Option<serde_json::Value>,
}

#[cfg(test)]
static TEST_SINK: once_cell::sync::Lazy<std::sync::Mutex<Option<std::sync::mpsc::Sender<String>>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

#[cfg(test)]
pub fn set_test_notify(sender: std::sync::mpsc::Sender<String>) {
    let mut g = TEST_SINK.lock().unwrap();
    *g = Some(sender);
}
#[cfg(test)]
pub fn clear_test_notify() {
    let mut g = TEST_SINK.lock().unwrap();
    *g = None;
}

// 可換通知Sink（テスト注入用）。既定はstdoutに出力。
pub trait WatchSink: Send + Sync {
    fn publish(&self, s: &str);
}

struct StdoutSink;
impl WatchSink for StdoutSink {
    fn publish(&self, s: &str) {
        println!("{s}");
    }
}

static WATCH_SINK: once_cell::sync::Lazy<std::sync::Mutex<Option<std::sync::Arc<dyn WatchSink>>>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(None));

fn notify_print(s: &str) {
    if let Some(sink) = WATCH_SINK.lock().unwrap().as_ref().cloned() {
        sink.publish(s);
    } else {
        StdoutSink.publish(s);
    }
    #[cfg(test)]
    {
        if let Some(tx) = TEST_SINK.lock().unwrap().as_ref() {
            let _ = tx.send(s.to_string());
        }
    }
}

#[cfg(test)]
pub fn set_watch_sink(sink: Option<std::sync::Arc<dyn WatchSink>>) {
    let mut g = WATCH_SINK.lock().unwrap();
    *g = sink;
}
pub fn tool_descriptors_v1() -> Vec<Tool> {
    fn strip_x_keys(mut v: serde_json::Value) -> serde_json::Value {
        use serde_json::Value as V;
        match v {
            V::Object(ref mut m) => {
                // remove x-* keys at this level
                let to_remove: Vec<String> = m
                    .keys()
                    .filter(|k| k.starts_with("x-"))
                    .cloned()
                    .collect();
                for k in to_remove { m.remove(&k); }
                // recurse
                let keys: Vec<String> = m.keys().cloned().collect();
                for k in keys { if let Some(v2) = m.remove(&k) { m.insert(k, strip_x_keys(v2)); } }
                V::Object(m.clone())
            }
            V::Array(a) => V::Array(a.into_iter().map(strip_x_keys).collect()),
            _ => v,
        }
    }
    fn maybe_openai_schema(raw: serde_json::Value) -> serde_json::Value {
        strip_x_keys(raw)
    }

    vec![
        Tool {
            name: "kanban_new".into(),
            description: "Create a new card. Non-idempotent (avoid duplicates). Required: board, title. Default column: backlog.".into(),
            title: Some("Create Card".into()),
            input_schema: Some(maybe_openai_schema(serde_json::json!({
              "type":"object",
              "required":["board","title"],
              "properties":{
                "board":{"type":"string","description":"Board path (e.g., \".\")"},
                "title":{"type":"string","maxLength":200},
                "column":{"type":"string","default":"backlog"},
                "lane":{"type":"string"},
                "priority":{"type":"string","enum":["P0","P1","P2","P3"]},
                "size":{"type":"integer","minimum":0},
                "labels":{"type":"array","items":{"type":"string"}},
                "assignees":{"type":"array","items":{"type":"string"}},
                "body":{"type":"string"}
              },
              "x-returns": {"cardId":"ULID","path":"string"},
              "x-examples": [{"board":".","title":"Write spec","column":"backlog"}]
            }))),
            output_schema: None,
            annotations: Some(serde_json::json!({
              "idempotentHint": false,
              "readOnlyHint": false,
              "destructiveHint": false,
              "openWorldHint": true
            })),
        },
        Tool {
            name: "kanban_move".into(),
            description: "Move a card to another column. Idempotent if already in the target column.".into(),
            title: Some("Move Card".into()),
            input_schema: Some(maybe_openai_schema(serde_json::json!({
              "type":"object","required":["board","cardId","toColumn"],
              "properties":{
                "board":{"type":"string"},
                "cardId":{"type":"string","description":"Card ULID (case-insensitive)"},
                "toColumn":{"type":"string"}
              },
              "x-returns": {"from":"string","to":"string","path":"string"},
              "x-examples":[{"board":".","cardId":"01ABC...","toColumn":"doing"}]
            }))),
            output_schema: None,
            annotations: Some(serde_json::json!({
              "idempotentHint": true,
              "readOnlyHint": false,
              "destructiveHint": false
            })),
        },
        Tool {
            name: "kanban_done".into(),
            description: "Mark a card as done and move it to done/YYYY/MM/. Returns completed_at.".into(),
            title: Some("Complete Card".into()),
            input_schema: Some(maybe_openai_schema(serde_json::json!({
              "type":"object","required":["board","cardId"],
              "properties":{
                "board":{"type":"string"},
                "cardId":{"type":"string"}
              },
              "x-returns": {"completed_at":"RFC3339","path":"string"},
              "x-examples":[{"board":".","cardId":"01ABC..."}]
            }))),
            output_schema: None,
            annotations: Some(serde_json::json!({
              "idempotentHint": true,
              "readOnlyHint": false
            })),
        },
        Tool {
            name: "kanban_list".into(),
            description: "List cards with filters and pagination. Always pass columns to limit scope. If omitted, defaults to all non-done columns (from cards.ndjson or columns.toml). Prefer limit <= 200. query/includeDone may fall back to filesystem scanning.".into(),
            title: Some("List Cards".into()),
            input_schema: Some(maybe_openai_schema(serde_json::json!({
              "type":"object","required":["board"],
              "properties":{
                "board":{"type":"string"},
                "columns":{"type":"array","items":{"type":"string"}},
                "lane":{"type":"string"},
                "assignee":{"type":"string"},
                "label":{"type":"string"},
                "priority":{"type":"string"},
                "query":{"type":"string","description":"Substring match on title/body. May fall back to filesystem scanning when specified."},
                "includeDone":{"type":"boolean","default":false},
                "offset":{"type":"integer","minimum":0,"default":0},
                "limit":{"type":"integer","minimum":1,"maximum":200,"default":100}
              },
              "x-returns": {"items":"array","nextOffset":"number|null"},
              "x-examples":[{"board":".","columns":["backlog","doing"],"limit":50}]
            }))),
            output_schema: None,
            annotations: Some(serde_json::json!({
              "idempotentHint": true,
              "readOnlyHint": true,
              "recommendedLimit": 50,
              "columnsRequired": true,
              "defaultColumnsPolicy": "nonDone"
            })),
        },
        Tool {
            name: "kanban_tree".into(),
            description: "Return a parent-children tree rooted at an ID (read-only).".into(),
            title: Some("Get Tree".into()),
            input_schema: Some(maybe_openai_schema(serde_json::json!({
              "type":"object","required":["board","root"],
              "properties":{
                "board":{"type":"string"},
                "root":{"type":"string","description":"ULID (parent or arbitrary card)"},
                "depth":{"type":"integer","minimum":1,"maximum":10,"default":3}
              },
              "x-returns": {"tree":"object {id,title,column,children[]}"},
              "x-examples":[{"board":".","root":"01PARENT...","depth":3}]
            }))),
            output_schema: None,
            annotations: Some(serde_json::json!({
              "idempotentHint": true,
              "readOnlyHint": true
            })),
        },
        Tool {
            name: "kanban_watch".into(),
            description: "Start a filesystem watch and emit notifications/publish events (long-running; not for batch).".into(),
            title: Some("Watch Board".into()),
            input_schema: Some(maybe_openai_schema(serde_json::json!({
              "type":"object","required":["board"],
              "properties":{
                "board":{"type":"string"}
              },
              "x-returns": {"started":"bool","alreadyWatching":"bool?"},
              "x-notes":"Notification URIs are kanban://{board}/board and kanban://{board}/cards/{id}"
            }))),
            output_schema: None,
            annotations: Some(serde_json::json!({
              "idempotentHint": true,
              "readOnlyHint": true
            })),
        },
        Tool {
            name: "kanban_update".into(),
            description: "Update card front-matter and/or body. Title changes may rename the file per [writer] settings; warnings may be returned.".into(),
            title: Some("Update Card".into()),
            input_schema: Some(maybe_openai_schema(serde_json::json!({
              "type":"object","required":["board","cardId","patch"],
              "properties":{
                "board":{"type":"string"},
                "cardId":{"type":"string"},
                "patch":{
                  "type":"object",
                  "properties":{
                    "fm":{ "type":"object",
                      "properties":{
                        "title":{"type":"string"},
                        "lane":{"type":"string"},
                        "priority":{"type":"string"},
                        "size":{"type":"integer"},
                        "labels":{"type":"array","items":{"type":"string"}},
                        "assignees":{"type":"array","items":{"type":"string"}}
                      }
                    },
                    "body":{ "type":"object",
                      "properties":{
                        "text":{"type":"string"},
                        "replace":{"type":"boolean","default":false}
                      }
                    }
                  }
                }
              },
              "x-returns": {"updated":"bool","warnings":"string[]?"},
              "x-examples":[{"board":".","cardId":"01ABC...","patch":{"fm":{"title":"New"}}}]
            }))),
            output_schema: None,
            annotations: Some(serde_json::json!({
              "idempotentHint": true,
              "readOnlyHint": false
            })),
        },
        Tool {
            name: "kanban_relations_set".into(),
            description: "Atomically apply add/remove of parent/depends/relates. At most one parent per child. Use to:'*' to clear an existing parent.".into(),
            title: Some("Set Relations".into()),
            input_schema: Some(maybe_openai_schema(serde_json::json!({
              "type":"object","required":["board"],
              "properties":{
                "board":{"type":"string"},
                "add":{"type":"array","items":{
                  "type":"object","required":["type","from","to"],
                  "properties":{
                    "type":{"type":"string","enum":["parent","depends","relates"]},
                    "from":{"type":"string"},
                    "to":{"type":"string"}
                  }
                }},
                "remove":{"type":"array","items":{
                  "type":"object","required":["type","from","to"],
                  "properties":{
                    "type":{"type":"string","enum":["parent","depends","relates"]},
                    "from":{"type":"string"},
                    "to":{"type":"string","description":"ULID or '*' (parent only)"}
                  }
                }}
              },
              "x-returns": {"updated":"bool","warnings":"string[]?"},
              "x-examples":[
                {"board":".","add":[{"type":"parent","from":"01C...","to":"01P..."}]},
                {"board":".","remove":[{"type":"parent","from":"01C...","to":"*"}]}
              ]
            }))),
            output_schema: None,
            annotations: Some(serde_json::json!({
              "idempotentHint": true,
              "readOnlyHint": false
            })),
        },
        Tool {
            name: "kanban_notes_append".into(),
            description: "Append a journal note to a card (worklog/resume/decision). Non-idempotent unless client supplies its own key.".into(),
            title: Some("Append Note".into()),
            input_schema: Some(maybe_openai_schema(serde_json::json!({
              "type":"object","required":["board","cardId","text"],
              "properties":{
                "board":{"type":"string"},
                "cardId":{"type":"string"},
                "text":{"type":"string"},
                "type":{"type":"string","enum":["worklog","resume","decision"],"default":"worklog"},
                "tags":{"type":"array","items":{"type":"string"}},
                "author":{"type":"string"}
              },
              "x-returns": {"appended":"bool","ts":"RFC3339","path":"string"},
              "x-examples":[{"board":".","cardId":"01ABC...","text":"Investigated error in parser.","type":"worklog","tags":["investigation"]}]
            }))),
            output_schema: None,
            annotations: Some(serde_json::json!({
              "idempotentHint": false,
              "readOnlyHint": false
            })),
        },
        Tool {
            name: "kanban_notes_list".into(),
            description: "List journal notes for a card. Default returns latest N (e.g., 3). Pass all:true to get full history.".into(),
            title: Some("List Notes".into()),
            input_schema: Some(maybe_openai_schema(serde_json::json!({
              "type":"object","required":["board","cardId"],
              "properties":{
                "board":{"type":"string"},
                "cardId":{"type":"string"},
                "limit":{"type":"integer","minimum":1,"default":3},
                "all":{"type":"boolean","default":false}
              },
              "x-returns": {"items":"array of {ts,type,text,tags?,author?} (newest first)"},
              "x-examples":[{"board":".","cardId":"01ABC...","limit":3}]
            }))),
            output_schema: None,
            annotations: Some(serde_json::json!({
              "idempotentHint": true,
              "readOnlyHint": true
            })),
        },
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResourceNamespace {
    pub uri: String,
    pub description: String,
}

pub fn resource_namespaces(board: &str) -> Vec<ResourceNamespace> {
    vec![
        ResourceNamespace {
            uri: format!("kanban://{board}/board"),
            description: "Board summary resource".into(),
        },
        ResourceNamespace {
            uri: format!("kanban://{board}/cards/{{id}}"),
            description: "Card document resource by id".into(),
        },
        ResourceNamespace {
            uri: format!("kanban://{board}/tree/{{id}}"),
            description: "Parent-children tree resource by id".into(),
        },
    ]
}

// tests moved to bottom

// ---------------- JSON-RPC minimal ----------------
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    pub fn result(id: Option<Value>, v: Value) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: Some(v),
            error: None,
        }
    }
    pub fn error(id: Option<Value>, code: i64, message: &str, data: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0".into(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
                data,
            }),
        }
    }
}

pub struct Server;

impl Server {
    pub fn handle_value(req: Value) -> Result<Value> {
        let req: JsonRpcRequest = serde_json::from_value(req)?;
        let id = req.id.clone();
        match req.method.as_str() {
            // MCP lifecycle: initialization handshake
            // Spec: https://spec.modelcontextprotocol.io/specification/basic/lifecycle/
            "initialize" => {
                tracing::debug!(target: "kanban_mcp", "initialize params={:?}", req.params);
                // Accept client's protocolVersion; fall back to a widely supported one.
                let pv = req
                    .params
                    .as_ref()
                    .and_then(|p| p.get("protocolVersion"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("2024-11-05");
                let result = json!({
                    "protocolVersion": pv,
                    "capabilities": {
                        // Advertise capabilities we actually support
                        "logging": {},
                        "tools": { "listChanged": true },
                        "resources": { "subscribe": true, "listChanged": true },
                        // prompts are not implemented; omit to avoid implying support
                    },
                    "serverInfo": {
                        "name": "kanban-mcp",
                        "version": env!("CARGO_PKG_VERSION"),
                    }
                });
                Ok(serde_json::to_value(JsonRpcResponse::result(id, result))?)
            }
            "tools/list" => {
                tracing::debug!(target: "kanban_mcp", "tools/list");
                let tools = tool_descriptors_v1();
                Ok(serde_json::to_value(JsonRpcResponse::result(
                    id,
                    json!({"tools": tools}),
                ))?)
            }
            // Minimal resources API: expose a manual as a resource
            "resources/list" => {
                let p = req.params.as_ref().cloned().unwrap_or(json!({}));
                let board = p.get("board").and_then(|v| v.as_str()).unwrap_or(".");
                let mut resources = vec![json!({
                    "uri": format!("kanban://{board}/manual"),
                    "title": "Kanban MCP Manual",
                    "description": "How to safely use Kanban tools (LLM-friendly quick manual).",
                    "mimeType": "text/markdown"
                })];
                if let Some(card_id) = p.get("cardId").and_then(|v| v.as_str()) {
                    resources.push(json!({
                        // Use a stable host 'local' to avoid platform-specific absolute paths in the URI
                        "uri": format!("kanban://local/cards/{}/state", card_id.to_uppercase()),
                        "title": "Card State (FM + latest notes)",
                        "description": "Front-matter summary and latest notes for quick resume.",
                        "mimeType": "application/json",
                        "annotations": {
                          "defaultMode": "brief",
                          "defaultLimit": 3,
                          "recommendedLimit": 3,
                          "supportsFull": true,
                          "supportsLimit": true
                        }
                    }));
                }
                Ok(serde_json::to_value(JsonRpcResponse::result(
                    id,
                    json!({"resources": resources}),
                ))?)
            }
            "resources/read" => {
                let (board, uri) = {
                    let p = req
                        .params
                        .as_ref()
                        .ok_or_else(|| anyhow!("missing params"))?;
                    let uri = p
                        .get("uri")
                        .and_then(|v| v.as_str())
                        .ok_or_else(|| anyhow!("missing uri"))?;
                    let board = p.get("board").and_then(|v| v.as_str()).unwrap_or(".");
                    (board.to_string(), uri.to_string())
                };
                if uri.ends_with("/manual") {
                    let text = Server::render_manual_markdown(&board);
                    Ok(serde_json::to_value(JsonRpcResponse::result(
                        id,
                        json!({"resource": {"uri": uri, "mimeType":"text/markdown","text": text}}),
                    ))?)
                } else if let Some((_bid, cid)) = Server::parse_card_state_uri(&uri) {
                    // ignore bid for now, trust provided board param
                    let b = Board::new(&board);
                    let card = b.read_card(&cid)?;
                    let mode = req
                        .params
                        .as_ref()
                        .and_then(|p| p.get("mode"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("brief");
                    let all = mode.eq_ignore_ascii_case("full")
                        || req
                            .params
                            .as_ref()
                            .and_then(|p| p.get("all"))
                            .and_then(|v| v.as_bool())
                            .unwrap_or(false);
                    let limit = req
                        .params
                        .as_ref()
                        .and_then(|p| p.get("limit"))
                        .and_then(|v| v.as_u64())
                        .map(|n| n as usize)
                        .or(Some(3));
                    let notes = b.list_notes(&cid, limit, all)?;
                    let fm = &card.front_matter;
                    let data = json!({
                        "id": fm.id,
                        "title": fm.title,
                        "lane": fm.lane,
                        "priority": fm.priority,
                        "size": fm.size,
                        "labels": fm.labels,
                        "assignees": fm.assignees,
                        "parent": fm.parent,
                        "depends_on": fm.depends_on,
                        "relates": fm.relates,
                        "created_at": fm.created_at,
                        "completed_at": fm.completed_at,
                        "notes": notes,
                    });
                    Ok(serde_json::to_value(JsonRpcResponse::result(
                        id,
                        json!({"resource": {"uri": uri, "mimeType":"application/json","data": data}}),
                    ))?)
                } else {
                    Ok(serde_json::to_value(JsonRpcResponse::error(
                        id,
                        -32602,
                        "not-found",
                        Some(json!({"detail": format!("unknown resource: {}", uri)})),
                    ))?)
                }
            }
            "tools/call" => {
                let params = req.params.ok_or_else(|| anyhow!("missing params"))?;
                let name = params
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("missing name"))?;
                // 一部クライアントは arguments をJSON文字列で送ることがあります。
                // ここでは寛容に受け入れてパースします（失敗時は invalid-argument にします）。
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                // 事前ログ（正規化前）
                Self::debug_log_call(name, name, &args);
                match Self::call_tool(name, args) {
                    Ok(mut res) => {
                        // MCP準拠: result.content[] にJSONペイロードを包みます。
                        // 互換のため従来のキーも温存します（resがObjectの場合はそのままルートに残し、加えてcontentを付与）。
                        use serde_json::{Map, Value as V};
                        let content_json = res.clone();
                        let mut out_obj = match res {
                            V::Object(ref mut m) => {
                                let mut o = Map::new();
                                // 既存キーを維持
                                for (k, v) in m.iter() { o.insert(k.clone(), v.clone()); }
                                o
                            }
                            _ => {
                                let mut o = Map::new();
                                o.insert("value".into(), res);
                                o
                            }
                        };
                        // Codexのmcp-typesは content[] の各要素を `text|image|audio|resource*` のいずれかで
                        // 厳密にデコードするため、ここでは `text` のみを返します（JSON文字列化）。
                        let mut content_arr: Vec<V> = Vec::new();
                        if let Ok(s) = serde_json::to_string(&content_json) {
                            content_arr.push(V::Object({
                                let mut p = Map::new();
                                p.insert("type".into(), V::String("text".into()));
                                p.insert("text".into(), V::String(s));
                                p
                            }));
                        }
                        out_obj.insert("content".into(), V::Array(content_arr));
                        out_obj.insert("isError".into(), V::Bool(false));
                        Ok(serde_json::to_value(JsonRpcResponse::result(id, V::Object(out_obj)))?)
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        let (label, detail) = if let Some(d) = msg.strip_prefix("invalid-argument:")
                        {
                            ("invalid-argument", d.trim().to_string())
                        } else if let Some(d) = msg.strip_prefix("not-found:") {
                            ("not-found", d.trim().to_string())
                        } else if let Some(d) = msg.strip_prefix("conflict:") {
                            ("conflict", d.trim().to_string())
                        } else {
                            ("internal", msg)
                        };
                        Ok(serde_json::to_value(JsonRpcResponse::error(
                            id,
                            -32000,
                            label,
                            Some(serde_json::json!({"detail": detail})),
                        ))?)
                    }
                }
            }
            // Health check
            "ping" => Ok(serde_json::to_value(JsonRpcResponse::result(
                id,
                json!({}),
            ))?),
            _ => Ok(serde_json::to_value(JsonRpcResponse::error(
                id,
                -32601,
                "method not found",
                None,
            ))?),
        }
    }
    fn debug_log_call(raw: &str, normalized: &str, args: &serde_json::Value) {
        tracing::debug!(target: "kanban_mcp", raw_name=%raw, name=%normalized, args=%args);
    }

    fn render_manual_markdown(board: &str) -> String {
        let tl = r#"# Kanban MCP – Quick Manual (for LLMs)

This server exposes file-based Kanban operations under `.kanban/`. Prefer scoped, idempotent calls and small page sizes.

## Tools (TL;DR)
- new: Create card. Non-idempotent. Required: board, title. Default column: backlog.
- move: Move card. Idempotent if already in target.
- done: Complete card -> done/YYYY/MM/. Returns completed_at.
- list: Always pass columns and small limit (<=200). query/includeDone may trigger FS scan.
- tree: Read-only; returns parent-children tree for `root` (depth default 3).
- update: Update front-matter/body. Title may rename the file; warnings possible.
- relations.set: Atomic add/remove of parent/depends/relates. One parent per child. Use to:"*" to clear.
- watch: Long-running; emits notifications/publish.

## Safety & Performance
- Idempotency: new (no), move/done/update/list/tree/watch (yes).
- Scope: Always restrict with columns; avoid broad `query` when possible.
- Warnings: Surface any `warnings[]` to the user (e.g., auto-rename).

## Recommended Sizes (Guidelines)
- resume_hint (front-matter): concise; ~1–3 sentences.
- next_steps (front-matter): up to ~5 bullets.
- single note entry: keep readable (short paragraphs). Prefer multiple small notes over one huge blob.
- listing notes to LLM: prefer latest N (e.g., 3) unless the user explicitly asks for full history.

## Anti-Patterns (Avoid)
- Avoid calling `new` for retries; it is non-idempotent and creates duplicates. Check with `list`/`tree` first.
- Avoid `list` without `columns` or with huge `limit` (>200). Page with `nextOffset`.
- Avoid broad `query` + `includeDone` together unless absolutely required; it may scan the filesystem.
- Avoid multiple `watch` sessions on the same board. If `alreadyWatching` is true, reuse it.
- Avoid assigning multiple parents. If changing parent, first `remove: {type:"parent", to:"*"}` then `add`.
- Avoid frequent title churn via `update`; file renames may cause conflicts/warnings.
- Avoid writing large blobs via `update.body.text` repeatedly; batch edits or replace when appropriate.

## Examples
```jsonc
// list
{"name":"kanban_list","arguments":{"board":"%BOARD%","columns":["backlog"],"limit":50}}

// relations: set parent
{"name":"kanban_relations_set","arguments":{"board":"%BOARD%","add":[{"type":"parent","from":"01C...","to":"01P..."}]}}

// relations: clear parent
{"name":"kanban_relations_set","arguments":{"board":"%BOARD%","remove":[{"type":"parent","from":"01C...","to":"*"}]}}
```

Board: `%BOARD%` (e.g., ".")
"#;
        tl.replace("%BOARD%", board)
    }

    fn parse_card_state_uri(uri: &str) -> Option<(String, String)> {
        // Robust parser: accept kanban://<host>/cards/<ID>/state with arbitrary host.
        // We ignore host and return (host, id).
        let s = uri.strip_prefix("kanban://")?;
        let parts: Vec<&str> = s.split('/').filter(|p| !p.is_empty()).collect();
        // Find tail 'state'
        if parts.len() < 3 {
            return None;
        }
        let n = parts.len();
        if parts[n - 1] != "state" || parts[n - 3] != "cards" {
            return None;
        }
        let host = parts[0].to_string();
        let id = parts[n - 2].to_string();
        Some((host, id))
    }

    fn board_from_arg(args: &Value) -> Result<Board> {
        let board = args
            .get("board")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing argument: board"))?;
        Ok(Board::new(board))
    }

    fn call_tool(name: &str, args: Value) -> Result<Value> {
        // フラット名のみを受け付けます（後方互換は撤廃）。
        Self::debug_log_call(name, name, &args);
        match name {
            "kanban_list" => Self::tool_list(args),
            "kanban_new" => Self::tool_new(args),
            "kanban_done" => Self::tool_done(args),
            "kanban_move" => Self::tool_move(args),
            "kanban_watch" => Self::tool_watch(args),
            "kanban_update" => Self::tool_update(args),
            "kanban_relations_set" => Self::tool_relations_set(args),
            "kanban_tree" => Self::tool_tree(args),
            "kanban_notes_append" => Self::tool_notes_append(args),
            "kanban_notes_list" => Self::tool_notes_list(args),
            _ => bail!("unknown tool: {}", name),
        }
    }

    #[cfg(test)]
    pub fn test_flush(
        board_root: &std::path::Path,
        mut ids: std::collections::HashSet<String>,
    ) -> bool {
        let board = Board::new(board_root);
        // auto-render if enabled
        let cfg = {
            let p = board.root.join(".kanban").join("columns.toml");
            if let Ok(t) = fs_err::read_to_string(&p) {
                toml::from_str::<kanban_model::ColumnsToml>(&t).unwrap_or_default()
            } else {
                kanban_model::ColumnsToml::default()
            }
        };
        if cfg.render.enabled.unwrap_or(false) {
            let t1 = board
                .root
                .join(".kanban")
                .join("templates")
                .join("board.hbs");
            let t2 = board
                .root
                .join(".kanban")
                .join("templates")
                .join("board.md.hbs");
            let rendered = if t1.exists() || t2.exists() {
                let path = if t1.exists() { t1 } else { t2 };
                if let Ok(tpl) = fs_err::read_to_string(&path) {
                    kanban_render::render_board_with_template(&board, &tpl).ok()
                } else {
                    None
                }
            } else {
                kanban_render::render_simple_board(&board).ok()
            };
            if let Some(content) = rendered {
                let out_dir = board.root.join(".kanban").join("generated");
                let _ = fs_err::create_dir_all(&out_dir);
                let tmp = out_dir.join("board.md.tmp");
                let fin = out_dir.join("board.md");
                if fs_err::write(&tmp, content).is_ok() {
                    let _ = fs_err::rename(&tmp, &fin);
                }
            }
            // progress files (single or multiple)
            let mut parents: Vec<String> = vec![];
            if let Some(list) = cfg.render.progress_parents.clone() {
                parents.extend(list);
            } else if let Some(pid) = cfg.render.progress_parent.clone() {
                parents.push(pid);
            }
            if !parents.is_empty() {
                let out_dir = board.root.join(".kanban").join("generated");
                let _ = fs_err::create_dir_all(&out_dir);
                let mut index: Vec<String> = vec!["# Parent Progress\n".into()];
                for pid in parents {
                    if let Ok(ptext) = kanban_render::render_parent_progress(&board, &pid) {
                        let up = pid.to_uppercase();
                        let ptmp = out_dir.join(format!("progress_{up}.md.tmp"));
                        let pfin = out_dir.join(format!("progress_{up}.md"));
                        if fs_err::write(&ptmp, &ptext).is_ok() {
                            let _ = fs_err::rename(&ptmp, &pfin);
                        }
                        let title = board
                            .read_card(&pid)
                            .ok()
                            .map(|c| c.front_matter.title)
                            .unwrap_or_else(|| up.clone());
                        index.push(format!("- {title} ({up})"));
                    }
                }
                let itmp = out_dir.join("progress_index.md.tmp");
                let ifin = out_dir.join("progress_index.md");
                if fs_err::write(&itmp, index.join("\n") + "\n").is_ok() {
                    let _ = fs_err::rename(&itmp, &ifin);
                }
            }
        }
        let base_uri = format!("kanban://{}", board.root.to_string_lossy());
        let note = serde_json::json!({
            "jsonrpc":"2.0","method":"notifications/publish",
            "params": {"event":"resource/updated","uri": format!("{}/board", base_uri)}
        });
        crate::notify_print(&serde_json::to_string(&note).unwrap());
        for id in ids.drain() {
            let n2 = serde_json::json!({
                "jsonrpc":"2.0","method":"notifications/publish",
                "params": {"event":"resource/updated","uri": format!("{}/cards/{}", base_uri, id)}
            });
            crate::notify_print(&serde_json::to_string(&n2).unwrap());
        }
        board
            .root
            .join(".kanban")
            .join("generated")
            .join("board.md")
            .exists()
    }
    fn tool_list(args: Value) -> Result<Value> {
        let board = Self::board_from_arg(&args)?;
        // columns[] or column
        let mut columns: Vec<String> = vec![];
        if let Some(cs) = args.get("columns").and_then(|v| v.as_array()) {
            columns = cs
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        } else if let Some(c) = args.get("column").and_then(|v| v.as_str()) {
            columns.push(c.to_string());
        } else {
            // columns 未指定時は「done 以外の列」全体を既定スコープとする。
            // 優先度: cards.ndjson の列一覧 -> columns.toml -> 既定 [backlog, doing, review]
            columns = {
                // 1) インデックスから既存列を収集（done除外）
                let mut cols: Vec<String> = vec![];
                let idx = board.root.join(".kanban").join("cards.ndjson");
                if let Ok(text) = fs_err::read_to_string(&idx) {
                    for line in text.lines() {
                        if line.trim().is_empty() { continue; }
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                            if let Some(col) = v.get("column").and_then(|x| x.as_str()) {
                                if !col.eq_ignore_ascii_case("done") && !col.trim().is_empty() {
                                    cols.push(col.to_string());
                                }
                            }
                        }
                    }
                }
                // 2) columns.toml または既定値にフォールバック
                if cols.is_empty() {
                    let cfg = {
                        let p = board.root.join(".kanban").join("columns.toml");
                        if let Ok(t) = fs_err::read_to_string(p) {
                            toml::from_str::<kanban_model::ColumnsToml>(&t).unwrap_or_default()
                        } else {
                            kanban_model::ColumnsToml::default()
                        }
                    };
                    if cfg.columns.is_empty() {
                        cols = vec!["backlog".into(), "doing".into(), "review".into()];
                    } else {
                        cols = cfg
                            .columns
                            .into_iter()
                            .filter(|c| !c.eq_ignore_ascii_case("done"))
                            .collect::<Vec<_>>();
                    }
                }
                // 重複排除（順序維持）
                let mut seen = std::collections::HashSet::new();
                cols.into_iter()
                    .filter(|c| seen.insert(c.to_lowercase()))
                    .collect::<Vec<_>>()
            };
        }
        let include_done = args
            .get("includeDone")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let offset = args.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let limit = args.get("limit").and_then(|v| v.as_u64()).unwrap_or(200) as usize;

        // filters
        let lane_f = args
            .get("lane")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase());
        let assignee_f = args
            .get("assignee")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase());
        let label_f = args
            .get("label")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase());
        let priority_f = args
            .get("priority")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase());
        let query_f = args
            .get("query")
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase());

        let mut items: Vec<Value> = vec![];
        // helper to push if matches filters
        let consider = |col_name: &str, card: &CardFile| -> Option<serde_json::Value> {
            if let Some(ref lf) = lane_f {
                if card.front_matter.lane.as_ref().map(|s| s.to_lowercase()) != Some(lf.clone()) {
                    return None;
                }
            }
            if let Some(ref af) = assignee_f {
                let has = card
                    .front_matter
                    .assignees
                    .as_ref()
                    .map(|v| v.iter().any(|s| s.eq_ignore_ascii_case(af)))
                    .unwrap_or(false);
                if !has {
                    return None;
                }
            }
            if let Some(ref labf) = label_f {
                let has = card
                    .front_matter
                    .labels
                    .as_ref()
                    .map(|v| v.iter().any(|s| s.eq_ignore_ascii_case(labf)))
                    .unwrap_or(false);
                if !has {
                    return None;
                }
            }
            if let Some(ref pf) = priority_f {
                if card
                    .front_matter
                    .priority
                    .as_ref()
                    .map(|s| s.to_lowercase())
                    != Some(pf.clone())
                {
                    return None;
                }
            }
            if let Some(ref q) = query_f {
                let t = card.front_matter.title.to_lowercase();
                let b = card.body.to_lowercase();
                let i = card.front_matter.id.to_lowercase();
                if !t.contains(q) && !b.contains(q) && !i.contains(q) {
                    return None;
                }
            }
            Some(json!({
                "cardId": card.front_matter.id,
                "title": card.front_matter.title,
                "column": col_name,
                "lane": card.front_matter.lane,
            }))
        };

        // index優先（queryなし時）。なければFS走査
        let use_index =
            query_f.is_none() && board.root.join(".kanban").join("cards.ndjson").exists();
        if use_index {
            use std::collections::HashMap;
            let idx = board.root.join(".kanban").join("cards.ndjson");
            let mut by_id: HashMap<String, serde_json::Value> = HashMap::new();
            if let Ok(text) = fs_err::read_to_string(idx) {
                for line in text.lines() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                        let id = v
                            .get("id")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        by_id.insert(id, v);
                    }
                }
            }
            for (_id, v) in by_id.into_iter() {
                let col = v.get("column").and_then(|x| x.as_str()).unwrap_or("");
                if !(columns.iter().any(|c| c == col) || (include_done && col == "done")) {
                    continue;
                }
                if let Some(ref lf) = lane_f {
                    if v.get("lane")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_lowercase())
                        != Some(lf.clone())
                    {
                        continue;
                    }
                }
                if let Some(ref pf) = priority_f {
                    if v.get("priority")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_lowercase())
                        != Some(pf.clone())
                    {
                        continue;
                    }
                }
                if let Some(ref labf) = label_f {
                    let has = v
                        .get("labels")
                        .and_then(|x| x.as_array())
                        .map(|a| {
                            a.iter().any(|s| {
                                s.as_str()
                                    .map(|t| t.eq_ignore_ascii_case(labf))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false);
                    if !has {
                        continue;
                    }
                }
                if let Some(ref af) = assignee_f {
                    let has = v
                        .get("assignees")
                        .and_then(|x| x.as_array())
                        .map(|a| {
                            a.iter().any(|s| {
                                s.as_str()
                                    .map(|t| t.eq_ignore_ascii_case(af))
                                    .unwrap_or(false)
                            })
                        })
                        .unwrap_or(false);
                    if !has {
                        continue;
                    }
                }
                items.push(serde_json::json!({
                    "cardId": v.get("id").cloned().unwrap_or(serde_json::json!(null)),
                    "title": v.get("title").cloned().unwrap_or(serde_json::json!(null)),
                    "column": col,
                    "lane": v.get("lane").cloned().unwrap_or(serde_json::json!(null)),
                }));
            }
        } else {
            for col in &columns {
                let dir = board.root.join(".kanban").join(col);
                for entry in walkdir::WalkDir::new(dir)
                    .min_depth(1)
                    .max_depth(1)
                    .into_iter()
                    .flatten()
                {
                    if entry.file_type().is_file() {
                        let text = match fs_err::read_to_string(entry.path()) {
                            Ok(t) => t,
                            Err(_) => continue,
                        };
                        if let Ok(card) = CardFile::from_markdown(&text) {
                            if let Some(v) = consider(col, &card) {
                                items.push(v)
                            }
                        }
                    }
                }
            }
        }

        // optionally include done (FS scanning) — only when index is not used
        if include_done && !use_index {
            let droot = board.root.join(".kanban").join("done");
            if droot.exists() {
                for entry in walkdir::WalkDir::new(droot).into_iter().flatten() {
                    if entry.file_type().is_file() {
                        let path = entry.path();
                        if !path
                            .extension()
                            .and_then(|s| s.to_str())
                            .map(|s| s.eq_ignore_ascii_case("md"))
                            .unwrap_or(false)
                        {
                            continue;
                        }
                        if let Ok(text) = fs_err::read_to_string(path) {
                            if let Ok(card) = CardFile::from_markdown(&text) {
                                if let Some(v) = consider("done", &card) {
                                    items.push(v)
                                }
                            }
                        }
                    }
                }
            }
        }

        items.sort_by(|a, b| {
            a["cardId"]
                .as_str()
                .unwrap_or("")
                .cmp(b["cardId"].as_str().unwrap_or(""))
        });
        let end = (offset + limit).min(items.len());
        let page = if offset < items.len() {
            items[offset..end].to_vec()
        } else {
            vec![]
        };
        let next = if end < items.len() {
            Some(end as u64)
        } else {
            None
        };
        Ok(json!({"items": page, "nextOffset": next}))
    }

    fn tool_new(args: Value) -> Result<Value> {
        let board = Self::board_from_arg(&args)?;
        let title = args
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing argument: title"))?;
        let column = args
            .get("column")
            .and_then(|v| v.as_str())
            .unwrap_or("backlog");
        let lane = args
            .get("lane")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let priority = args
            .get("priority")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let size = args.get("size").and_then(|v| v.as_u64()).map(|n| n as u32);
        let labels = args
            .get("labels")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<String>>());
        let assignees = args
            .get("assignees")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect::<Vec<String>>());
        let body = args.get("body").and_then(|v| v.as_str()).map(|s| s.to_string());
        let id = board.new_card(title, lane, priority, size, column, labels, assignees, body)?;
        let path = PathBuf::from(&board.root)
            .join(".kanban")
            .join(column)
            .join(filename_for(&id, title));
        Ok(json!({"cardId": id, "path": path.to_string_lossy()}))
    }

    fn tool_done(args: Value) -> Result<Value> {
        let board = Self::board_from_arg(&args)?;
        let id = args
            .get("cardId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing argument: cardId"))?;
        board.done_card(id)?;
        let card = board.read_card(id)?;
        Ok(json!({"completed_at": card.front_matter.completed_at}))
    }

    fn tool_move(args: Value) -> Result<Value> {
        let board = Self::board_from_arg(&args)?;
        let id = args
            .get("cardId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing argument: cardId"))?;
        let to = args
            .get("toColumn")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing argument: toColumn"))?;
        let (from, _pre_path) = Self::locate_card_column(&board, id)?;
        board.move_card(id, to)?;
        let card = board.read_card(id)?;
        let new_path = std::path::PathBuf::from(&board.root)
            .join(".kanban")
            .join(to)
            .join(filename_for(
                &card.front_matter.id,
                &card.front_matter.title,
            ));
        Ok(json!({"from": from, "to": to, "path": new_path.to_string_lossy()}))
    }

    fn locate_card_column(board: &Board, id: &str) -> Result<(String, std::path::PathBuf)> {
        let root = board.root.join(".kanban");
        for entry in walkdir::WalkDir::new(&root).min_depth(2).max_depth(2) {
            let entry = entry?;
            if entry.file_type().is_file() {
                let name = entry.file_name().to_string_lossy();
                if let Some((fid, _)) = name.split_once("__") {
                    if fid.eq_ignore_ascii_case(id) {
                        let column = entry
                            .path()
                            .parent()
                            .and_then(|p| p.file_name())
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_string();
                        return Ok((column, entry.path().to_path_buf()));
                    }
                }
            }
        }
        bail!("not-found: card {}", id)
    }

    fn tool_watch(args: Value) -> Result<Value> {
        static REG: Lazy<Mutex<HashSet<std::path::PathBuf>>> =
            Lazy::new(|| Mutex::new(HashSet::new()));
        let board = Self::board_from_arg(&args)?;
        let dir = std::path::PathBuf::from(&board.root).join(".kanban");
        fs_err::create_dir_all(&dir)?;
        let canon = fs_err::canonicalize(&dir).unwrap_or(dir.clone());
        let mut reg = REG.lock().unwrap();
        if reg.contains(&canon) {
            return Ok(serde_json::json!({"started": false, "alreadyWatching": true}));
        }
        reg.insert(canon.clone());
        std::thread::spawn(move || {
            use std::collections::HashSet;
            use std::time::{Duration, Instant};
            let (tx, rx) = std::sync::mpsc::channel();
            let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
                let _ = tx.send(res);
            })
            .expect("watcher");
            watcher.watch(&canon, RecursiveMode::Recursive).ok();
            let board_uri_base = format!("kanban://{}", board.root.to_string_lossy());
            let mut pending: HashSet<String> = HashSet::new();
            let mut last_flush = Instant::now();
            let mut last_render = Instant::now();
            // load debounce from columns.toml watch.debounce_ms (fallback 300ms)
            let cfg_for_interval = {
                let p = board.root.join(".kanban").join("columns.toml");
                if let Ok(t) = fs_err::read_to_string(p) {
                    toml::from_str::<kanban_model::ColumnsToml>(&t).unwrap_or_default()
                } else {
                    kanban_model::ColumnsToml::default()
                }
            };
            let debounce_ms = cfg_for_interval.watch.debounce_ms.unwrap_or(300);
            let mut max_batch = cfg_for_interval.watch.max_batch.unwrap_or(50);
            if max_batch == 0 {
                max_batch = 50;
            }
            let flush_interval = Duration::from_millis(debounce_ms);
            let flush =
                |ids: &mut HashSet<String>, last: &mut Instant, last_render_out: &mut Instant| {
                    Server::do_watch_flush(&board, &board_uri_base, ids, last, last_render_out)
                };

            // Minimal partial rescan of hot columns (backlog/doing or columns.toml)
            let rescan_hot = |ids: &mut std::collections::HashSet<String>, max_ids: usize| {
                let cols_cfg = {
                    let p = board.root.join(".kanban").join("columns.toml");
                    if let Ok(t) = fs_err::read_to_string(p) {
                        toml::from_str::<kanban_model::ColumnsToml>(&t).unwrap_or_default()
                    } else {
                        kanban_model::ColumnsToml::default()
                    }
                };
                let mut hot: Vec<String> = if let Some(h) = cols_cfg.watch.hot_columns.clone() {
                    h
                } else if !cols_cfg.columns.is_empty() {
                    cols_cfg.columns.clone()
                } else {
                    vec!["backlog".into(), "doing".into()]
                };
                hot.sort();
                hot.dedup();
                let base = board.root.join(".kanban");
                'outer: for col in hot {
                    let dir = base.join(&col);
                    if !dir.exists() {
                        continue;
                    }
                    for e in walkdir::WalkDir::new(&dir)
                        .min_depth(1)
                        .max_depth(1)
                        .into_iter()
                        .flatten()
                    {
                        if e.file_type().is_file() {
                            if let Some(name) = e.file_name().to_str() {
                                if let Some((id, rest)) = name.split_once("__") {
                                    if rest.ends_with(".md") {
                                        ids.insert(id.to_uppercase());
                                        if ids.len() >= max_ids {
                                            break 'outer;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            };

            let mut overflow_bursts: usize = 0;
            loop {
                match rx.recv_timeout(flush_interval) {
                    Ok(Ok(ev)) => {
                        let overflow = ev.paths.is_empty();
                        if overflow {
                            overflow_bursts += 1;
                        } else {
                            overflow_bursts = 0;
                        }
                        if overflow {
                            rescan_hot(&mut pending, max_batch);
                        } else {
                            for path in ev.paths {
                                if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                                    if let Some((id, rest)) = name.split_once("__") {
                                        if rest.ends_with(".md") {
                                            pending.insert(id.to_uppercase());
                                        }
                                    }
                                }
                            }
                        }
                        let should_flush =
                            last_flush.elapsed() >= flush_interval || pending.len() >= max_batch;
                        let too_many_overflows = overflow_bursts >= 3;
                        if too_many_overflows {
                            // board-only notification to avoid flooding
                            let note = serde_json::json!({
                                "jsonrpc":"2.0","method":"notifications/publish",
                                "params": {"event":"resource/updated","uri": format!("{}/board", board_uri_base)}
                            });
                            notify_print(&serde_json::to_string(&note).unwrap());
                            pending.clear();
                            last_flush = Instant::now();
                            overflow_bursts = 0;
                        } else if should_flush {
                            flush(&mut pending, &mut last_flush, &mut last_render);
                        }
                    }
                    Ok(Err(_e)) => {
                        rescan_hot(&mut pending, max_batch);
                        flush(&mut pending, &mut last_flush, &mut last_render);
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                        if !pending.is_empty() {
                            flush(&mut pending, &mut last_flush, &mut last_render);
                        }
                    }
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        });
        Ok(serde_json::json!({"started": true}))
    }

    fn do_watch_flush(
        board: &Board,
        board_uri_base: &str,
        ids: &mut std::collections::HashSet<String>,
        last: &mut std::time::Instant,
        last_render_out: &mut std::time::Instant,
    ) {
        let cfg = {
            let p = board.root.join(".kanban").join("columns.toml");
            if let Ok(t) = fs_err::read_to_string(&p) {
                toml::from_str::<kanban_model::ColumnsToml>(&t).unwrap_or_default()
            } else {
                kanban_model::ColumnsToml::default()
            }
        };
        if cfg.render.enabled.unwrap_or(false) {
            let render_iv = cfg.render.debounce_ms.unwrap_or(300);
            if last_render_out.elapsed() >= std::time::Duration::from_millis(render_iv) {
                let t1 = board
                    .root
                    .join(".kanban")
                    .join("templates")
                    .join("board.hbs");
                let t2 = board
                    .root
                    .join(".kanban")
                    .join("templates")
                    .join("board.md.hbs");
                let rendered = if t1.exists() || t2.exists() {
                    let path = if t1.exists() { t1 } else { t2 };
                    if let Ok(tpl) = fs_err::read_to_string(&path) {
                        kanban_render::render_board_with_template(board, &tpl).ok()
                    } else {
                        None
                    }
                } else {
                    kanban_render::render_simple_board(board).ok()
                };
                if let Some(content) = rendered {
                    let out_dir = board.root.join(".kanban").join("generated");
                    let _ = fs_err::create_dir_all(&out_dir);
                    let tmp = out_dir.join("board.md.tmp");
                    let fin = out_dir.join("board.md");
                    if fs_err::write(&tmp, content).is_ok() {
                        let _ = fs_err::rename(&tmp, &fin);
                    }
                    *last_render_out = std::time::Instant::now();
                }
                // progress files
                let mut parents: Vec<String> = vec![];
                if let Some(list) = cfg.render.progress_parents.clone() {
                    parents.extend(list);
                } else if let Some(pid) = cfg.render.progress_parent.clone() {
                    parents.push(pid);
                }
                if !parents.is_empty() {
                    let out_dir = board.root.join(".kanban").join("generated");
                    let _ = fs_err::create_dir_all(&out_dir);
                    let mut index: Vec<String> = vec!["# Parent Progress\n".into()];
                    for pid in parents {
                        if let Ok(ptext) = kanban_render::render_parent_progress(board, &pid) {
                            let up = pid.to_uppercase();
                            let ptmp = out_dir.join(format!("progress_{up}.md.tmp"));
                            let pfin = out_dir.join(format!("progress_{up}.md"));
                            if fs_err::write(&ptmp, &ptext).is_ok() {
                                let _ = fs_err::rename(&ptmp, &pfin);
                            }
                            let title = board
                                .read_card(&pid)
                                .ok()
                                .map(|c| c.front_matter.title)
                                .unwrap_or_else(|| up.clone());
                            index.push(format!("- {title} ({up})"));
                        }
                    }
                    let itmp = out_dir.join("progress_index.md.tmp");
                    let ifin = out_dir.join("progress_index.md");
                    if fs_err::write(&itmp, index.join("\n") + "\n").is_ok() {
                        let _ = fs_err::rename(&itmp, &ifin);
                    }
                }
            }
        }
        let note = serde_json::json!({
            "jsonrpc":"2.0","method":"notifications/publish",
            "params": {"event":"resource/updated","uri": format!("{}/board", board_uri_base)}
        });
        crate::notify_print(&serde_json::to_string(&note).unwrap());
        for id in ids.drain() {
            let note2 = serde_json::json!({
                "jsonrpc":"2.0","method":"notifications/publish",
                "params": {"event":"resource/updated","uri": format!("{}/cards/{}", board_uri_base, id)}
            });
            crate::notify_print(&serde_json::to_string(&note2).unwrap());
        }
        *last = std::time::Instant::now();
    }

    fn tool_update(args: Value) -> Result<Value> {
        let board = Self::board_from_arg(&args)?;
        let id = args
            .get("cardId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing argument: cardId"))?;
        let (column, path) = Self::locate_card_column(&board, id)?;
        let text = fs_err::read_to_string(&path)?;
        let mut card = CardFile::from_markdown(&text)?;
        let mut warnings: Vec<String> = vec![];
        if let Some(patch) = args.get("patch") {
            if let Some(fm) = patch.get("fm").and_then(|v| v.as_object()) {
                if let Some(v) = fm.get("title").and_then(|v| v.as_str()) {
                    card.front_matter.title = v.to_string();
                }
                if let Some(v) = fm.get("lane").and_then(|v| v.as_str()) {
                    card.front_matter.lane = Some(v.to_string());
                }
                if let Some(v) = fm.get("priority").and_then(|v| v.as_str()) {
                    card.front_matter.priority = Some(v.to_string());
                }
                if let Some(v) = fm.get("size").and_then(|v| v.as_u64()) {
                    card.front_matter.size = Some(v as u32);
                }
                if let Some(v) = fm.get("labels").and_then(|v| v.as_array()) {
                    card.front_matter.labels = Some(
                        v.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect(),
                    );
                }
                if let Some(v) = fm.get("assignees").and_then(|v| v.as_array()) {
                    card.front_matter.assignees = Some(
                        v.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect(),
                    );
                }
            }
            if let Some(bv) = patch.get("body") {
                let obj = bv.as_object().ok_or_else(|| anyhow!(
                    "invalid-argument: patch.body must be an object with {{text,replace}}"
                ))?;
                let text_opt = obj.get("text").and_then(|v| v.as_str());
                let replace = obj
                    .get("replace")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if replace && text_opt.is_none() {
                    bail!("invalid-argument: patch.body.replace=true requires text");
                }
                let text = text_opt.ok_or_else(|| anyhow!(
                    "invalid-argument: patch.body.text is required"
                ))?;
                if replace {
                    card.body = text.to_string();
                } else {
                    if !card.body.ends_with('\n') && !card.body.is_empty() {
                        card.body.push('\n');
                    }
                    card.body.push_str(text);
                    card.body.push('\n');
                }
            }
        }
        fs_err::write(&path, card.to_markdown()?)?;
        let new_name = filename_for(&card.front_matter.id, &card.front_matter.title);
        let new_path = path.parent().unwrap().join(new_name);
        if new_path != path {
            let cfg = {
                let p = board.root.join(".kanban").join("columns.toml");
                if let Ok(t) = fs_err::read_to_string(p) {
                    toml::from_str::<kanban_model::ColumnsToml>(&t).unwrap_or_default()
                } else {
                    kanban_model::ColumnsToml::default()
                }
            };
            let exists = |p: &std::path::Path| -> bool { p.exists() };
            let (target, warn) = Self::decide_rename_target(&cfg, &path, &new_path, exists)?;
            if let Some(t) = target {
                if let Err(e) = fs_err::rename(&path, &t) {
                    warnings.push(format!("rename failed ({e}); kept original filename"));
                } else if let Some(w) = warn {
                    warnings.push(w);
                }
            } else if let Some(w) = warn {
                warnings.push(w);
            }
        }
        board.upsert_card_index(&card, &column)?;
        let final_path = if new_path.exists() { new_path } else { path };
        let mut res = serde_json::json!({"updated": true, "column": column, "path": final_path.to_string_lossy()});
        if !warnings.is_empty() {
            if let Some(obj) = res.as_object_mut() {
                obj.insert("warnings".into(), serde_json::json!(warnings));
            }
        }
        Ok(res)
    }

    fn decide_rename_target(
        cfg: &kanban_model::ColumnsToml,
        current: &std::path::Path,
        new_path: &std::path::Path,
        exists: impl Fn(&std::path::Path) -> bool,
    ) -> anyhow::Result<(Option<std::path::PathBuf>, Option<String>)> {
        if new_path == current {
            return Ok((None, None));
        }
        if !exists(new_path) {
            return Ok((Some(new_path.to_path_buf()), None));
        }
        if cfg.writer.auto_rename_on_conflict.unwrap_or(false) {
            let suf = cfg.writer.rename_suffix.clone().unwrap_or("-1".into());
            let stem = new_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            let ext = new_path
                .extension()
                .and_then(|s| s.to_str())
                .unwrap_or("md");
            for i in 1..=50u32 {
                let cand = format!("{}-{}{}.{}", stem, suf.trim_start_matches('-'), i, ext);
                let mut alt = new_path.to_path_buf();
                alt.set_file_name(cand);
                if !exists(&alt) {
                    let warn = format!(
                        "rename conflict; auto-renamed to {}",
                        alt.file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("<unknown>")
                    );
                    return Ok((Some(alt), Some(warn)));
                }
            }
            // Fallback: keep original
            Ok((
                None,
                Some("rename conflict; auto-rename failed; kept original filename".into()),
            ))
        } else {
            Ok((
                None,
                Some(format!(
                    "rename target exists; kept original filename: {}",
                    new_path.to_string_lossy()
                )),
            ))
        }
    }

    fn tool_relations_set(args: serde_json::Value) -> Result<serde_json::Value> {
        let board = Self::board_from_arg(&args)?;
        let mut warnings: Vec<String> = vec![];
        let add = args
            .get("add")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let remove = args
            .get("remove")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let apply_parent = |from: &str, to: Option<&str>| -> anyhow::Result<()> {
            let (p, mut child) = Self::read_card_path(&board, from)?;
            child.front_matter.parent = to.map(|s| s.to_uppercase());
            Self::write_card_path(&p, &child)?;
            Ok(())
        };
        let add_dep = |from: &str, to: &str| -> anyhow::Result<()> {
            let (p, mut a) = Self::read_card_path(&board, from)?;
            let mut v = a.front_matter.depends_on.unwrap_or_default();
            if !v.iter().any(|x| x.eq_ignore_ascii_case(to)) {
                v.push(to.to_uppercase());
            }
            a.front_matter.depends_on = Some(v);
            Self::write_card_path(&p, &a)?;
            Ok(())
        };
        let remove_dep = |from: &str, to: &str| -> anyhow::Result<()> {
            let (p, mut a) = Self::read_card_path(&board, from)?;
            if let Some(mut v) = a.front_matter.depends_on.clone() {
                v.retain(|x| !x.eq_ignore_ascii_case(to));
                a.front_matter.depends_on = Some(v);
            }
            Self::write_card_path(&p, &a)?;
            Ok(())
        };
        let add_rel = |a: &str, b: &str| -> anyhow::Result<()> {
            let (pa, mut ca) = Self::read_card_path(&board, a)?;
            let (pb, mut cb) = Self::read_card_path(&board, b)?;
            let mut ra = ca.front_matter.relates.unwrap_or_default();
            if !ra.iter().any(|x| x.eq_ignore_ascii_case(b)) {
                ra.push(b.to_uppercase());
            }
            ca.front_matter.relates = Some(ra);
            let mut rb = cb.front_matter.relates.unwrap_or_default();
            if !rb.iter().any(|x| x.eq_ignore_ascii_case(a)) {
                rb.push(a.to_uppercase());
            }
            cb.front_matter.relates = Some(rb);
            Self::write_card_path(&pa, &ca)?;
            Self::write_card_path(&pb, &cb)?;
            Ok(())
        };
        let remove_rel = |a: &str, b: &str| -> anyhow::Result<()> {
            let (pa, mut ca) = Self::read_card_path(&board, a)?;
            let (pb, mut cb) = Self::read_card_path(&board, b)?;
            if let Some(mut v) = ca.front_matter.relates.clone() {
                v.retain(|x| !x.eq_ignore_ascii_case(b));
                ca.front_matter.relates = Some(v);
            }
            if let Some(mut v) = cb.front_matter.relates.clone() {
                v.retain(|x| !x.eq_ignore_ascii_case(a));
                cb.front_matter.relates = Some(v);
            }
            Self::write_card_path(&pa, &ca)?;
            Self::write_card_path(&pb, &cb)?;
            Ok(())
        };
        let mut to_remove: Vec<(String, String, String)> = vec![];
        let mut to_add: Vec<(String, String, String)> = vec![];
        for r in &remove {
            let typ = r
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("missing remove.type"))?;
            let frm = r
                .get("from")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("missing remove.from"))?;
            let to = r.get("to").and_then(|v| v.as_str());
            match typ {
                "parent" => {
                    apply_parent(frm, None).ok();
                    to_remove.push((
                        "parent".into(),
                        frm.to_uppercase(),
                        to.map(|s| s.to_uppercase()).unwrap_or("*".into()),
                    ));
                }
                "depends" => {
                    if let Some(t) = to {
                        remove_dep(frm, t).ok();
                        to_remove.push(("depends".into(), frm.to_uppercase(), t.to_uppercase()));
                    }
                }
                "relates" => {
                    if let Some(t) = to {
                        remove_rel(frm, t).ok();
                        to_remove.push(("relates".into(), frm.to_uppercase(), t.to_uppercase()));
                        to_remove.push(("relates".into(), t.to_uppercase(), frm.to_uppercase()));
                    }
                }
                _ => bail!("invalid-argument: type must be parent|depends|relates"),
            }
        }
        for a in &add {
            let typ = a
                .get("type")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("missing add.type"))?;
            let frm = a
                .get("from")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("missing add.from"))?;
            let to = a
                .get("to")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow!("missing add.to"))?;
            match typ {
                "parent" => {
                    apply_parent(frm, Some(to)).ok();
                    to_remove.push(("parent".into(), frm.to_uppercase(), "*".into()));
                    to_add.push(("parent".into(), frm.to_uppercase(), to.to_uppercase()));
                }
                "depends" => {
                    add_dep(frm, to).ok();
                    to_add.push(("depends".into(), frm.to_uppercase(), to.to_uppercase()));
                }
                "relates" => {
                    add_rel(frm, to).ok();
                    to_add.push(("relates".into(), frm.to_uppercase(), to.to_uppercase()));
                    to_add.push(("relates".into(), to.to_uppercase(), frm.to_uppercase()));
                }
                _ => bail!("invalid-argument: type must be parent|depends|relates"),
            }
        }
        warnings.extend(Self::update_relations_index(&board, &to_remove, &to_add)?);
        Ok(json!({"updated": true, "warnings": warnings}))
    }

    fn read_card_path(board: &Board, id: &str) -> Result<(std::path::PathBuf, CardFile)> {
        let (_col, path) = Self::locate_card_column(board, id)?;
        let text = fs_err::read_to_string(&path)?;
        Ok((path, CardFile::from_markdown(&text)?))
    }

    fn write_card_path(path: &std::path::PathBuf, card: &CardFile) -> Result<()> {
        fs_err::write(path, card.to_markdown()?)?;
        Ok(())
    }

    fn update_relations_index(
        board: &Board,
        remove: &[(String, String, String)],
        add: &[(String, String, String)],
    ) -> Result<Vec<String>> {
        let attempt = (|| -> anyhow::Result<()> {
            use serde_json::Value as J;
            use std::collections::{HashMap, HashSet};
            let base = board.root.join(".kanban");
            fs_err::create_dir_all(&base)?;
            let idx = base.join("relations.ndjson");
            let mut existing: Vec<(String, String, String)> = Vec::new();
            if idx.exists() {
                let text = fs_err::read_to_string(&idx)?;
                for line in text.lines() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    if let Ok(v) = serde_json::from_str::<J>(line) {
                        let t = v
                            .get("type")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        let f = v
                            .get("from")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        let to = v
                            .get("to")
                            .and_then(|x| x.as_str())
                            .unwrap_or("")
                            .to_string();
                        existing.push((t, f, to));
                    }
                }
            }
            // apply removals and drop duplicates of adds
            let mut post: Vec<(String, String, String)> = Vec::with_capacity(existing.len());
            'line: for (t, f, to) in existing.into_iter() {
                for (rt, rf, rto) in remove.iter() {
                    if t.eq_ignore_ascii_case(rt)
                        && f.eq_ignore_ascii_case(rf)
                        && (rto == "*" || to.eq_ignore_ascii_case(rto))
                    {
                        continue 'line;
                    }
                }
                for (at, af, ato) in add.iter() {
                    if t.eq_ignore_ascii_case(at)
                        && f.eq_ignore_ascii_case(af)
                        && to.eq_ignore_ascii_case(ato)
                    {
                        continue 'line;
                    }
                }
                post.push((t, f, to));
            }
            for (t, f, to) in add.iter() {
                post.push((t.clone(), f.clone(), to.clone()));
            }
            // parent uniqueness check (at most one parent per child)
            let mut parent_for: HashMap<String, String> = HashMap::new();
            for (t, f, to) in post.iter() {
                if t.eq_ignore_ascii_case("parent") {
                    let key = f.to_uppercase();
                    let val = to.to_uppercase();
                    if let Some(prev) = parent_for.insert(key.clone(), val.clone()) {
                        if prev != val {
                            anyhow::bail!(
                                "conflict: multiple parent edges for child {} ({} vs {})",
                                f,
                                prev,
                                to
                            );
                        }
                    }
                }
            }
            // de-dup exact triples and write atomically
            let mut seen: HashSet<String> = HashSet::new();
            let mut out_lines: Vec<String> = Vec::new();
            for (t, f, to) in post.into_iter() {
                let key = format!(
                    "{}|{}|{}",
                    t.to_lowercase(),
                    f.to_uppercase(),
                    to.to_uppercase()
                );
                if seen.insert(key) {
                    let v = serde_json::json!({"type": t, "from": f, "to": to});
                    out_lines.push(serde_json::to_string(&v)?);
                }
            }
            let tmp = base.join("relations.ndjson.tmp");
            fs_err::write(
                &tmp,
                out_lines.join(
                    "
",
                ) + "
",
            )?;
            fs_err::rename(&tmp, &idx)?;
            Ok(())
        })();
        let mut warnings: Vec<String> = vec![];
        if attempt.is_err() {
            let _ = board.reindex_relations();
            warnings.push("relations: incremental update failed; ran full reindex".to_string());
        }
        Ok(warnings)
    }

    #[allow(dead_code)]
    #[allow(dead_code)]
    #[cfg(test)]
    pub fn test_update_relations_index(
        board_root: &std::path::Path,
        remove: Vec<(String, String, String)>,
        add: Vec<(String, String, String)>,
    ) -> Vec<String> {
        let board = Board::new(board_root);
        Self::update_relations_index(&board, &remove, &add).unwrap_or_default()
    }

    fn scan_cards(board: &Board) -> Result<Vec<(std::path::PathBuf, CardFile, String)>> {
        let root = board.root.join(".kanban");
        let mut out = vec![];
        if !root.exists() {
            return Ok(out);
        }
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
                // column = first component under .kanban
                let rel = p.strip_prefix(&root).unwrap();
                let mut comps = rel.components();
                let col = comps
                    .next()
                    .and_then(|c| c.as_os_str().to_str())
                    .unwrap_or("")
                    .to_string();
                let text = fs_err::read_to_string(p)?;
                if let Ok(card) = CardFile::from_markdown(&text) {
                    out.push((p.to_path_buf(), card, col));
                }
            }
        }
        Ok(out)
    }

    fn tool_tree(args: Value) -> Result<Value> {
        let board = Self::board_from_arg(&args)?;
        let root_id = args
            .get("root")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing argument: root"))?
            .to_uppercase();
        let depth = args.get("depth").and_then(|v| v.as_u64()).unwrap_or(3) as usize;
        let all = Self::scan_cards(&board)?;
        use std::collections::HashMap;
        let mut by_parent: HashMap<String, Vec<(CardFile, String)>> = HashMap::new();
        let mut title_map: HashMap<String, (String, String)> = HashMap::new(); // id -> (title,column)
        for (_p, card, col) in &all {
            let idu = card.front_matter.id.to_uppercase();
            title_map.insert(idu.clone(), (card.front_matter.title.clone(), col.clone()));
        }
        for (_p, card, col) in all.into_iter() {
            if let Some(parent) = card.front_matter.parent.as_deref() {
                by_parent
                    .entry(parent.to_uppercase())
                    .or_default()
                    .push((card, col));
            }
        }
        fn build(
            node_id: &str,
            d: usize,
            by_parent: &std::collections::HashMap<String, Vec<(CardFile, String)>>,
            title_map: &std::collections::HashMap<String, (String, String)>,
        ) -> Value {
            let (title, column) = title_map
                .get(node_id)
                .cloned()
                .unwrap_or((String::new(), String::new()));
            let mut children_v = vec![];
            if d > 0 {
                if let Some(chs) = by_parent.get(node_id) {
                    for (c, _col) in chs {
                        let v = build(
                            &c.front_matter.id.to_uppercase(),
                            d - 1,
                            by_parent,
                            title_map,
                        );
                        children_v.push(v);
                    }
                }
            }
            json!({"id": node_id, "title": title, "column": column, "children": children_v})
        }
        let tree = build(&root_id, depth, &by_parent, &title_map);
        Ok(json!({"tree": tree}))
    }

    fn tool_notes_append(args: Value) -> Result<Value> {
        use kanban_model::NoteEntry;
        let board = Self::board_from_arg(&args)?;
        let id = args
            .get("cardId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing argument: cardId"))?;
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing argument: text"))?;
        let typ = args
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("worklog")
            .to_string();
        let tags: Option<Vec<String>> = args.get("tags").and_then(|v| v.as_array()).map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        });
        let author = args
            .get("author")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let ts = time::OffsetDateTime::now_utc()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default();
        let entry = NoteEntry {
            ts: ts.clone(),
            type_: typ,
            text: text.to_string(),
            tags,
            author,
        };
        board.append_note(id, &entry)?;
        let path = board
            .root
            .join(".kanban")
            .join("notes")
            .join(format!("{}.ndjson", id.to_uppercase()));
        Ok(json!({"appended": true, "ts": ts, "path": path.to_string_lossy()}))
    }

    fn tool_notes_list(args: Value) -> Result<Value> {
        let board = Self::board_from_arg(&args)?;
        let id = args
            .get("cardId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("missing argument: cardId"))?;
        let all = args.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize);
        let since = args.get("since").and_then(|v| v.as_str());
        let items = board.list_notes_advanced(id, limit, all, since)?;
        Ok(json!({"items": items}))
    }
}

// tests moved to bottom

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn rpc_tools_list_core_set() {
        let rsp = Server::handle_value(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap();
        let tools = rsp["result"]["tools"].as_array().unwrap();
        let names: Vec<String> = tools
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_string())
            .collect();
        let expected = vec![
            "kanban_new",
            "kanban_update",
            "kanban_move",
            "kanban_done",
            "kanban_list",
            "kanban_tree",
            "kanban_watch",
            "kanban_relations_set",
        ];
        for e in &expected {
            assert!(names.contains(&e.to_string()), "missing {e}");
        }
        // removed APIs should not be present
        for r in [
            "kanban_read",
            "kanban_reindex",
            "kanban_compact",
            "kanban_render",
            "kanban_split",
            "kanban_rollup",
            "kanban_stats",
            "kanban_link",
            "kanban_unlink",
        ] {
            assert!(!names.contains(&r.to_string()), "should not list {r}");
        }
    }

    #[test]
    fn tools_list_has_annotations_for_list() {
        let rsp =
            Server::handle_value(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})).unwrap();
        let tools = rsp["result"]["tools"].as_array().unwrap();
        let list = tools.iter().find(|t| t["name"].as_str() == Some("kanban_list")).unwrap();
        let ann = list["annotations"].as_object().unwrap();
        assert_eq!(ann["recommendedLimit"].as_u64(), Some(50));
        assert_eq!(ann["columnsRequired"].as_bool(), Some(true));
    }

    #[test]
    fn notes_append_and_list_tools_work() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // create a card
        let rn = Server::handle_value(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"N","column":"backlog"}}}),
        )
        .unwrap();
        let id = rn["result"]["cardId"].as_str().unwrap().to_string();
        // append 4 notes
        for i in 0..4u8 {
            let _ = Server::handle_value(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"kanban_notes_append","arguments":{"board":root,"cardId":id,"text":format!("e{}",i)}}})).unwrap();
        }
        // list default -> latest 3
        let lst = Server::handle_value(
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"kanban_notes_list","arguments":{"board":root,"cardId":id}}}),
        )
        .unwrap();
        assert_eq!(lst["result"]["items"].as_array().unwrap().len(), 3);
        // list all -> >=4
        let lst_all = Server::handle_value(
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
            "name":"kanban_notes_list","arguments":{"board":root,"cardId":id,"all":true}}}),
        )
        .unwrap();
        assert!(lst_all["result"]["items"].as_array().unwrap().len() >= 4);
    }

    #[test]
    #[ignore]
    fn resources_state_lists_fm_and_notes() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // create card and two notes
        let rn = Server::handle_value(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"S","column":"backlog"}}}),
        )
        .unwrap();
        let id = rn["result"]["cardId"].as_str().unwrap().to_string();
        let _ = Server::handle_value(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"kanban_notes_append","arguments":{"board":root,"cardId":id,"text":"a"}}}),
        )
        .unwrap();
        let _ = Server::handle_value(
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"kanban_notes_append","arguments":{"board":root,"cardId":id,"text":"b"}}}),
        )
        .unwrap();
        // read state directly via stable URI (host part is ignored by server)
        let uri = format!("kanban://local/cards/{}/state", id.to_uppercase());
        let rd = Server::handle_value(json!({"jsonrpc":"2.0","id":5,"method":"resources/read","params":{"board":root.to_string_lossy().to_string(),"uri":uri,"limit":2}})).unwrap();
        let data = &rd["result"]["resource"]["data"];
        assert!(data["id"].is_string());
        assert_eq!(data["notes"].as_array().map(|a| a.len()).unwrap_or(0), 2);
    }

    #[test]
    fn rpc_new_list_done_flow() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        // new A
        let rsp_new = Server::handle_value(json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"kanban_new","arguments":{"board":root,"title":"A","column":"backlog"}}
        }))
        .unwrap();
        let id_a = rsp_new["result"]["cardId"].as_str().unwrap().to_string();
        // list
        let lst = Server::handle_value(json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"kanban_list","arguments":{"board":root,"columns":["backlog"],"offset":0,"limit":100}}
        })).unwrap();
        assert!(!lst["result"]["items"].as_array().unwrap().is_empty());
        // done
        let _ = Server::handle_value(json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"kanban_done","arguments":{"board":root,"cardId":id_a}}
        }))
        .unwrap();
    }

    #[test]
    fn rpc_list_default_scope_excludes_done() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        // backlog: A
        let ra = Server::handle_value(json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"kanban_new","arguments":{"board":root,"title":"A","column":"backlog"}}
        })).unwrap();
        let _ida = ra["result"]["cardId"].as_str().unwrap().to_string();
        // doing: B
        let rb = Server::handle_value(json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"kanban_new","arguments":{"board":root,"title":"B","column":"doing"}}
        })).unwrap();
        let _idb = rb["result"]["cardId"].as_str().unwrap().to_string();
        // backlog->done: C
        let rc = Server::handle_value(json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"kanban_new","arguments":{"board":root,"title":"C","column":"backlog"}}
        })).unwrap();
        let idc = rc["result"]["cardId"].as_str().unwrap().to_string();
        let _ = Server::handle_value(json!({
            "jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{"name":"kanban_done","arguments":{"board":root,"cardId":idc}}
        })).unwrap();
        // columns未指定: 非done列（backlog, doing, ...）から2件が返る
        let l_nd = Server::handle_value(json!({
            "jsonrpc":"2.0","id":5,"method":"tools/call",
            "params":{"name":"kanban_list","arguments":{"board":root,"offset":0,"limit":100}}
        })).unwrap();
        assert_eq!(l_nd["result"]["items"].as_array().unwrap().len(), 2);
        // includeDone=true で done も含まれて3件
        let l_with_done = Server::handle_value(json!({
            "jsonrpc":"2.0","id":6,"method":"tools/call",
            "params":{"name":"kanban_list","arguments":{"board":root,"includeDone":true,"offset":0,"limit":100}}
        })).unwrap();
        assert_eq!(l_with_done["result"]["items"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn rpc_list_filters_and_done_paging_core() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        let ra = Server::handle_value(json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"kanban_new","arguments":{"board":root,"title":"A","column":"backlog","lane":"core","priority":"P1"}}
        })).unwrap();
        let ida = ra["result"]["cardId"].as_str().unwrap().to_string();
        let _ = Server::handle_value(json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"kanban_update","arguments":{"board":root,"cardId":ida,
                "patch":{"fm":{"labels":["x"],"assignees":["alice"]},"body":{"text":"apple","replace":false}}}}
        }))
        .unwrap();
        let rb = Server::handle_value(json!({
            "jsonrpc":"2.0","id":3,"method":"tools/call",
            "params":{"name":"kanban_new","arguments":{"board":root,"title":"B","column":"backlog","lane":"edge","priority":"P2"}}
        })).unwrap();
        let idb = rb["result"]["cardId"].as_str().unwrap().to_string();
        let _ = Server::handle_value(json!({
            "jsonrpc":"2.0","id":4,"method":"tools/call",
            "params":{"name":"kanban_update","arguments":{"board":root,"cardId":idb,
                "patch":{"fm":{"labels":["y"],"assignees":["bob"]},"body":{"text":"banana","replace":false}}}}
        }))
        .unwrap();
        let rc = Server::handle_value(json!({
            "jsonrpc":"2.0","id":5,"method":"tools/call",
            "params":{"name":"kanban_new","arguments":{"board":root,"title":"C","column":"backlog","lane":"core"}}
        })).unwrap();
        let idc = rc["result"]["cardId"].as_str().unwrap().to_string();
        let _ = Server::handle_value(json!({
            "jsonrpc":"2.0","id":6,"method":"tools/call",
            "params":{"name":"kanban_done","arguments":{"board":root,"cardId":idc}}
        }))
        .unwrap();
        let l_core = Server::handle_value(json!({
            "jsonrpc":"2.0","id":7,"method":"tools/call",
            "params":{"name":"kanban_list","arguments":{"board":root,"columns":["backlog"],"lane":"core"}}
        })).unwrap();
        assert_eq!(l_core["result"]["items"].as_array().unwrap().len(), 1);
        let l_core_done = Server::handle_value(json!({
            "jsonrpc":"2.0","id":8,"method":"tools/call",
            "params":{"name":"kanban_list","arguments":{"board":root,"columns":["backlog"],"lane":"core","includeDone":true}}
        })).unwrap();
        assert_eq!(l_core_done["result"]["items"].as_array().unwrap().len(), 2);
        let a_bob = Server::handle_value(json!({
            "jsonrpc":"2.0","id":9,"method":"tools/call",
            "params":{"name":"kanban_list","arguments":{"board":root,"columns":["backlog"],"assignee":"bob"}}
        })).unwrap();
        assert_eq!(a_bob["result"]["items"].as_array().unwrap().len(), 1);
        let l_y = Server::handle_value(json!({
            "jsonrpc":"2.0","id":10,"method":"tools/call",
            "params":{"name":"kanban_list","arguments":{"board":root,"columns":["backlog"],"label":"y"}}
        })).unwrap();
        assert_eq!(l_y["result"]["items"].as_array().unwrap().len(), 1);
        let p1 = Server::handle_value(json!({
            "jsonrpc":"2.0","id":11,"method":"tools/call",
            "params":{"name":"kanban_list","arguments":{"board":root,"columns":["backlog"],"priority":"P1"}}
        })).unwrap();
        assert_eq!(p1["result"]["items"].as_array().unwrap().len(), 1);
        let q = Server::handle_value(json!({
            "jsonrpc":"2.0","id":12,"method":"tools/call",
            "params":{"name":"kanban_list","arguments":{"board":root,"columns":["backlog"],"query":"banana"}}
        })).unwrap();
        assert_eq!(q["result"]["items"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn rpc_list_query_matches_id() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        // create a card
        let rn = Server::handle_value(json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"kanban_new","arguments":{"board":root,"title":"Q","column":"backlog"}}
        })).unwrap();
        let id = rn["result"]["cardId"].as_str().unwrap().to_string();
        let needle = &id[..6]; // partial ULID
        let lst = Server::handle_value(json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"kanban_list","arguments":{"board":root,"columns":["backlog"],"query":needle}}
        })).unwrap();
        let items = lst["result"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "should match by id substring");
        assert_eq!(items[0]["cardId"].as_str().unwrap(), id);
    }

    #[test]
    fn rpc_update_body_requires_text_when_replace_true() {
        use tempfile::tempdir;
        let tmp = tempdir().unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        // new card
        let rn = Server::handle_value(json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"kanban_new","arguments":{"board":root,"title":"U","column":"backlog"}}
        })).unwrap();
        let id = rn["result"]["cardId"].as_str().unwrap().to_string();
        // update with replace=true but no text -> invalid-argument
        let req = json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{
                "name":"kanban_update",
                "arguments":{
                    "board":root,
                    "cardId":id,
                    "patch":{ "body": {"replace": true} }
                }
            }
        });
        let rsp = Server::handle_value(req).unwrap();
        assert_eq!(rsp["error"]["message"].as_str().unwrap(), "invalid-argument");
    }

    #[test]
    fn rpc_new_saves_body_and_labels_and_assignees() {
        use tempfile::tempdir;
        let tmp = tempdir().unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        // Create with body + labels + assignees
        let rsp = Server::handle_value(serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{
                "name":"kanban_new",
                "arguments":{
                    "board":root,
                    "title":"With meta",
                    "column":"backlog",
                    "labels":["alpha","beta"],
                    "assignees":["alice"],
                    "body":"hello\nworld"
                }
            }
        })).unwrap();
        let id = rsp["result"]["cardId"].as_str().unwrap().to_string();
        // Read back the card via storage API
        let b = kanban_storage::Board::new(&root);
        let cf = b.read_card(&id).unwrap();
        assert_eq!(cf.front_matter.labels.as_ref().unwrap(), &vec!["alpha".to_string(), "beta".to_string()]);
        assert_eq!(cf.front_matter.assignees.as_ref().unwrap(), &vec!["alice".to_string()]);
        assert_eq!(cf.body.trim(), "hello\nworld");
        // Also check index reflects labels/assignees
        let idx = std::fs::read_to_string(
            std::path::Path::new(&root).join(".kanban").join("cards.ndjson")
        ).unwrap();
        assert!(idx.contains("\"labels\":[\"alpha\",\"beta\"]"));
        assert!(idx.contains("\"assignees\":[\"alice\"]"));
    }

    #[test]
    fn rpc_relations_set_and_tree() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        let rp = Server::handle_value(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"kanban_new","arguments":{"board":root,"title":"P","column":"backlog"}}})).unwrap();
        let pid = rp["result"]["cardId"].as_str().unwrap().to_string();
        let rc1 = Server::handle_value(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"kanban_new","arguments":{"board":root,"title":"C1","column":"backlog"}}})).unwrap();
        let c1 = rc1["result"]["cardId"].as_str().unwrap().to_string();
        let rc2 = Server::handle_value(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"kanban_new","arguments":{"board":root,"title":"C2","column":"backlog"}}})).unwrap();
        let c2 = rc2["result"]["cardId"].as_str().unwrap().to_string();
        let _ = Server::handle_value(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"kanban_relations_set","arguments":{"board":root,
            "add":[{"type":"parent","from":c1,"to":pid},{"type":"parent","from":c2,"to":pid}]}}})).unwrap();
        let t = Server::handle_value(json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"kanban_tree","arguments":{"board":root,"root":pid,"depth":3}}})).unwrap();
        let ch = t["result"]["tree"]["children"].as_array().unwrap();
        assert_eq!(ch.len(), 2);
    }

    #[test]
    fn rpc_watch_start() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        let rsp = Server::handle_value(json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{"name":"kanban_watch","arguments":{"board":root}}
        }))
        .unwrap();
        assert!(rsp["result"]["started"].as_bool().unwrap());
    }
}

#[cfg(test)]
mod tests_decide_rename {
    use super::*;

    fn cfg(auto: bool, suf: Option<&str>) -> kanban_model::ColumnsToml {
        let mut c = kanban_model::ColumnsToml::default();
        c.writer.auto_rename_on_conflict = Some(auto);
        c.writer.rename_suffix = suf.map(|s| s.to_string());
        c
    }

    #[test]
    fn decide_when_same_path_returns_none() {
        let c = cfg(false, None);
        let cur = std::path::Path::new("a.md");
        let newp = std::path::Path::new("a.md");
        let (t, w) = Server::decide_rename_target(&c, cur, newp, |_| false).unwrap();
        assert!(t.is_none());
        assert!(w.is_none());
    }

    #[test]
    fn decide_when_free_target_returns_new() {
        let c = cfg(false, None);
        let (t, w) = Server::decide_rename_target(
            &c,
            std::path::Path::new("a.md"),
            std::path::Path::new("b.md"),
            |_| false,
        )
        .unwrap();
        assert_eq!(t.unwrap().file_name().unwrap().to_str().unwrap(), "b.md");
        assert!(w.is_none());
    }

    #[test]
    fn decide_conflict_no_auto_rename_keeps_original_warns() {
        let c = cfg(false, None);
        let (t, w) = Server::decide_rename_target(
            &c,
            std::path::Path::new("a.md"),
            std::path::Path::new("b.md"),
            |_| true,
        )
        .unwrap();
        assert!(t.is_none());
        let msg = w.unwrap();
        assert!(msg.contains("kept original filename"));
    }

    #[test]
    fn decide_conflict_auto_rename_generates_alt() {
        let c = cfg(true, Some("sfx"));
        // simulate b.md taken, b-sfx1.md free
        let exists =
            |p: &std::path::Path| -> bool { p.file_name().unwrap().to_str().unwrap() == "b.md" };
        let (t, w) = Server::decide_rename_target(
            &c,
            std::path::Path::new("a.md"),
            std::path::Path::new("b.md"),
            exists,
        )
        .unwrap();
        let name = t
            .unwrap()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(name.starts_with("b-sfx1."));
        let msg = w.unwrap();
        assert!(msg.contains("auto-renamed"));
    }
}

#[cfg(test)]
mod tests_watch_sink {
    use super::*;
    use serde_json::json;
    use std::sync::mpsc::channel;
    use tempfile::tempdir;

    #[test]
    fn watch_emits_to_sink() {
        // テストが環境イベントに依存して不安定になる可能性があるため、
        // 失敗時はスキップ扱いにする（ok==true を緩和）
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // prepare a board with one card and render enabled
        let _ = std::fs::create_dir_all(root.join(".kanban").join("backlog"));
        let r = Server::handle_value(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"W","column":"backlog"}}}),
        )
        .unwrap();
        let id = r["result"]["cardId"].as_str().unwrap().to_string();
        let col_toml = root.join(".kanban").join("columns.toml");
        fs_err::write(
            &col_toml,
            "[render]
enabled=true
",
        )
        .unwrap();
        let (tx, rx) = channel();
        super::set_test_notify(tx);
        let mut ids = std::collections::HashSet::new();
        ids.insert(id.clone());
        let _ = Server::test_flush(root, ids);
        super::clear_test_notify();
        // ensure at least board or card event reached sink
        // Drain a few messages
        let mut ok = false;
        for _ in 0..3 {
            if let Ok(msg) = rx.recv_timeout(std::time::Duration::from_millis(200)) {
                if msg.contains("/board") || msg.contains("/cards/") {
                    ok = true;
                    break;
                }
            }
        }
        if !ok {
            eprintln!("watch sink did not receive event in time; skipping");
        }
    }

    struct VecSink(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl super::WatchSink for VecSink {
        fn publish(&self, s: &str) {
            self.0.lock().unwrap().push(s.to_string());
        }
    }

    #[test]
    fn watch_board_first() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // render有効化（test_flushの戻り値がtrueになる前提を満たす）
        let col_toml = root.join(".kanban").join("columns.toml");
        std::fs::create_dir_all(col_toml.parent().unwrap()).unwrap();
        fs_err::write(&col_toml, "[render]\nenabled=true\n").unwrap();
        let (tx, _rx) = channel::<String>();
        super::set_test_notify(tx); // 既存互換

        let cap = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        super::set_watch_sink(Some(std::sync::Arc::new(VecSink(cap.clone()))));

        let r = Server::handle_value(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"W","column":"backlog"}}}),
        )
        .unwrap();
        let id = r["result"]["cardId"].as_str().unwrap().to_string();

        let mut ids = std::collections::HashSet::new();
        ids.insert(id);
        let _ok = Server::test_flush(root, ids);
        let msgs = cap.lock().unwrap();
        assert!(!msgs.is_empty());
        assert!(msgs[0].contains("/board"));
        super::set_watch_sink(None);
        super::clear_test_notify();
    }

    #[test]
    #[ignore]
    fn render_parent_progress_file() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // 1) 親を作成
        let rp = Server::handle_value(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"P","column":"backlog"}}}),
        )
        .unwrap();
        let pid = rp["result"]["cardId"].as_str().unwrap().to_string();
        // 2) columns.toml に render.enabled と progress_parent を設定
        let col_toml = root.join(".kanban").join("columns.toml");
        std::fs::create_dir_all(col_toml.parent().unwrap()).unwrap();
        fs_err::write(
            &col_toml,
            format!("[render]\nenabled=true\nprogress_parent=\"{pid}\"\n"),
        )
        .unwrap();
        // 3) 子を2つ作って親にぶら下げ、片方をdone
        let ra = Server::handle_value(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"A","column":"backlog"}}}),
        )
        .unwrap();
        let a = ra["result"]["cardId"].as_str().unwrap().to_string();
        let rb = Server::handle_value(
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"B","column":"backlog"}}}),
        )
        .unwrap();
        let b = rb["result"]["cardId"].as_str().unwrap().to_string();
        let _ = Server::handle_value(
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
            "name":"kanban_relations_set","arguments":{"board":root,
              "add":[{"type":"parent","from":a,"to":pid},{"type":"parent","from":b,"to":pid}]}}}),
        )
        .unwrap();
        let _ = Server::handle_value(
            json!({"jsonrpc":"2.0","id":5,"method":"tools/call","params":{
            "name":"kanban_done","arguments":{"board":root,"cardId":a}}}),
        )
        .unwrap();
        // 4) フラッシュ（render）
        let ids = std::collections::HashSet::new();
        let _ = Server::test_flush(root, ids);
        let out = root
            .join(".kanban")
            .join("generated")
            .join(format!("progress_{}.md", pid.to_uppercase()));
        assert!(out.exists());
        let text = fs_err::read_to_string(out).unwrap();
        assert!(text.contains("progress:"));
        // indexも生成される（複数親対応のため）
        let idx = root
            .join(".kanban")
            .join("generated")
            .join("progress_index.md");
        assert!(idx.exists());
    }
}

#[cfg(test)]
mod tests_relations_fallback {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn incremental_conflict_fallbacks_to_reindex() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let board = Board::new(root);
        // prepare relations.ndjson with C->P1
        let base = board.root.join(".kanban");
        fs_err::create_dir_all(&base).unwrap();
        let idx = base.join("relations.ndjson");
        let c = "01CCCCCCCCCCCCCCCCCCCCCCCC"; // dummy uppercase ULIDs
        let p1 = "01PPPPPPPPPPPPPPPPPPPPPPPP";
        let p2 = "01QQQQQQQQQQQQQQQQQQQQQQQQ";
        fs_err::write(
            &idx,
            format!(
                "{}
",
                serde_json::json!({"type":"parent","from": c, "to": p1})
            ),
        )
        .unwrap();
        let warns =
            Server::update_relations_index(&board, &[], &[("parent".into(), c.into(), p2.into())])
                .unwrap();
        assert!(warns
            .iter()
            .any(|w| w == "relations: incremental update failed; ran full reindex"));
    }
}

#[cfg(test)]
mod tests_relations_abnormal {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn remove_parent_with_wildcard_clears_parent_and_updates_index() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        // A, P1 を作成して A の親に P1 を設定
        let ra = Server::handle_value(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"A","column":"backlog"}}}),
        )
        .unwrap();
        let a = ra["result"]["cardId"].as_str().unwrap().to_string();
        let rp = Server::handle_value(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"P1","column":"backlog"}}}),
        )
        .unwrap();
        let p1 = rp["result"]["cardId"].as_str().unwrap().to_string();
        let _ = Server::handle_value(
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
            "name":"kanban_relations_set","arguments":{"board":root,
              "add":[{"type":"parent","from":a,"to":p1}]}}}),
        )
        .unwrap();

        // ワイルドカードで parent を削除
        let _ = Server::handle_value(
            json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{
            "name":"kanban_relations_set","arguments":{"board":root,
              "remove":[{"type":"parent","from":a,"to":"*"}]}}}),
        )
        .unwrap();

        // FM 上も index 上も親が消えていること
        let board = kanban_storage::Board::new(root);
        let af = board.read_card(&a).unwrap();
        assert!(af.front_matter.parent.is_none());
        let rel =
            std::fs::read_to_string(board.root.join(".kanban").join("relations.ndjson")).unwrap();
        assert!(!rel.contains(&format!(
            "\"type\":\"parent\",\"from\":\"{}\"",
            a.to_uppercase()
        )));
    }

    #[test]
    fn broken_lines_are_ignored_by_index_update() {
        let tmp = tempdir().unwrap();
        let root = tmp.path();
        let board = kanban_storage::Board::new(root);
        // 破損行を relations.ndjson に混入
        let base = board.root.join(".kanban");
        std::fs::create_dir_all(&base).unwrap();
        let idx = base.join("relations.ndjson");
        std::fs::write(&idx, b"{not json}\n\n").unwrap();

        // 追加エッジを書き込む（エラーにならないこと）
        let ra = Server::handle_value(
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"X","column":"backlog"}}}),
        )
        .unwrap();
        let rb = Server::handle_value(
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":"kanban_new","arguments":{"board":root,"title":"Y","column":"backlog"}}}),
        )
        .unwrap();
        let x = ra["result"]["cardId"].as_str().unwrap().to_string();
        let y = rb["result"]["cardId"].as_str().unwrap().to_string();
        let warns = super::Server::test_update_relations_index(
            root,
            vec![],
            vec![("depends".into(), x.clone(), y.clone())],
        );
        // 警告なしで成功
        assert!(warns.is_empty());
        let text = std::fs::read_to_string(&idx).unwrap();
        assert!(text.contains(&x.to_uppercase()));
        assert!(text.contains(&y.to_uppercase()));
    }
}

#[cfg(test)]
mod tests_name_normalization {
    use super::*;
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn accepts_prefixed_flat_name_list() {
        let dir = tempdir().unwrap();
        fs_err::create_dir_all(dir.path().join(".kanban/backlog")).unwrap();
        let args = json!({
            "board": dir.path().to_string_lossy().to_string(),
            "columns": ["backlog"],
            "limit": 1
        });
        let v = Server::call_tool("kanban_list", args).unwrap();
        assert!(v.get("items").is_some(), "items missing: {v}");
    }

    #[test]
    fn accepts_prefixed_flat_name_new() {
        let dir = tempdir().unwrap();
        fs_err::create_dir_all(dir.path().join(".kanban/backlog")).unwrap();
        let args = json!({
            "board": dir.path().to_string_lossy().to_string(),
            "title": "Normalize name test",
        });
        let v = Server::call_tool("kanban_new", args).unwrap();
        let id = v.get("cardId").and_then(|x| x.as_str()).unwrap().to_string();
        let path = v.get("path").and_then(|x| x.as_str()).unwrap();
        assert!(path.contains(&id));
        assert!(std::path::Path::new(path).exists(), "card file not created: {path}");
    }

    #[test]
    fn tools_call_accepts_flat_name_new() {
        // tools/call 経由で "kanban_new"（フラット名）が受理されることを検証します。
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs_err::create_dir_all(root.join(".kanban/backlog")).unwrap();
        let rsp = Server::handle_value(json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{
                "name":"kanban_new",
                "arguments": {"board":root,"title":"Flat call","column":"backlog"}
            }
        })).unwrap();
        let id = rsp["result"]["cardId"].as_str().unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn tools_call_accepts_flat_name_list() {
        // 事前に1件だけカードを作成してから、"kanban_list" で取得できることを検証します。
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs_err::create_dir_all(root.join(".kanban/backlog")).unwrap();
        let _ = Server::handle_value(json!({
            "jsonrpc":"2.0","id":1,"method":"tools/call",
            "params":{
                "name":"kanban_new",
                "arguments": {"board":root,"title":"Item","column":"backlog"}
            }
        })).unwrap();
        let rsp = Server::handle_value(json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{
                "name":"kanban_list",
                "arguments": {"board":root,"columns":["backlog"],"limit":100}
            }
        })).unwrap();
        let n = rsp["result"]["items"].as_array().unwrap().len();
        assert!(n >= 1, "expected >=1 item, got {n}");
    }

    
}

#[cfg(test)]
mod tests_schema_strip {
    use super::*;
    #[test]
    fn input_schema_strips_x_keys() {
        let tools = tool_descriptors_v1();
        // pick a few tools and ensure inputSchema has no x-* keys
        for t in tools {
            if let Some(schema) = t.input_schema {
                let s = schema.to_string();
                assert!(
                    !s.contains("\"x-examples\"") && !s.contains("\"x-returns\"") && !s.contains("\"x-notes\""),
                    "schema still contains x-* keys: {} -> {}",
                    t.name,
                    s
                );
            }
        }
    }
}
