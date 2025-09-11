---
title: Codex CLI + kanban-mcp 連携ノート
---

# Codex CLI + kanban-mcp 連携ノート

目的: Codex CLI（OpenAI製）から kanban-mcp (stdio) を利用する際の設定と既知事象を共有する。

## 動作確認済みの前提
- kanban バイナリ: `target/release/kanban`
- 起動: `kanban mcp --board <REPO> --log-level debug --openai`
- OpenAI互換におけるツール一覧 (`tools/list`): ツール名は `kanban_list` 等のフラット名を返す。
- tools/call の応答: MCP準拠の `result.content[]`（`json`/`text`）＋互換のため従来キー（`items` など）も温存。

## Codex 設定（~/.codex/config.toml）例
```toml
[mcp_servers.kanban]
type    = "stdio"
command = "/absolute/path/to/kanban"
args    = ["mcp", "--board", "/absolute/path/to/your/repo", "--log-level", "debug", "--openai"]
```

## 再現手順（codex exec）
```
echo 'Kanban MCPサーバが有効なら、tools/list (name: kanban_list) を board="${PWD}", columns=["backlog"], limit=1 で1回だけ実行し、得られたJSONのresult部分のみを最後に表示して下さい。余計な説明は出力しないで下さい。' \
  | codex exec --json -C "$PWD" --skip-git-repo-check --full-auto -
```

## 既知事象（2025-09-09 時点）
- Codex側のイベント出力に `mcp_tool_call_end ... Err: tool call error: tool call failed` が表示される場合があるが、
  kanban-mcp のサーバログを見ると該当リクエストに正しく成功応答（JSON-RPC 200/`result.content[]`）を返している。
  このため Codex UI/仲介層が応答の取り扱いに失敗している可能性が疑われる。

### サーバ側ログ例（成功応答／STDERR）
```
[REQ] {"id":3,"jsonrpc":"2.0","method":"tools/call","params":{"name":"kanban_list","arguments":{"board":"/repo","columns":["backlog"],"limit":1}}}
[RSP] {"id":3,"jsonrpc":"2.0","result":{"content":[{"type":"text","text":"{\"items\":[...],\"nextOffset\":1}"}],"isError":false,"items":[...],"nextOffset":1}}
```

## ワークアラウンド
- 直接 `kanban mcp` にJSONを流し込む（`tools/call` 直叩き）方式は安定して動作する。
- Codexから使う場合は `--openai` を必ず付ける（OpenAIツール名/スキーマ制約が厳格なため）。

## 提案（upstream向け）
- `tools/call` の `result.content[]` に `json` と同内容の `text` を併記しても、Codexの一部経路で `tool call error` になる事象がある。
- `~/.codex/logs/kanban-mcp.log` と `codex exec --json` のイベントログで、server側成功・UI側エラーの食い違いが再現する。
