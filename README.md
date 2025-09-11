---
title: kanban-mcp（ファイルKanban + MCP + CLI）
---

# kanban-mcp

このリポジトリは、`.kanban/`ディレクトリを真実のソースとして扱う「ファイルベースKanban」を、MCPサーバ（JSON‑RPC/stdio）とCLIで安全に運用するための実装です。

## 最短起動（Quick Start）
- 前提: Rust 1.88+、`cargo`が利用可能であること。
- ビルド/テスト:
```
cargo build --workspace --all-targets
cargo test  --workspace --all-targets --no-fail-fast
```
- ボード作成とMCP起動（カレント配下）:
```
mkdir -p .kanban/backlog
kanban mcp --board . --log-level info
```
- ツール一覧（別端末から）:
```
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | kanban mcp --board .
```

## ビルド方法（Build）
- 前提: Rust 1.88+（`rustup`推奨）
- 手順:
```
cargo build --workspace --release
# 生成物: target/release/kanban
```
- 動作確認:
```
target/release/kanban mcp --board .
```

## Codex CLI への導入（MCP Server / TOML）
Codex CLI は `~/.codex/config.toml` でMCPサーバを宣言できます。TOMLではトップレベルキーが `mcp_servers`（snake_case）である点に注意してください（JSONでは `mcpServers`）。

`~/.codex/config.toml` の例:
```toml
# IMPORTANT: top-level key is `mcp_servers` (TOML)
[mcp_servers.kanban]
command = "/absolute/path/to/kanban"         # 例: /usr/local/bin/kanban
args    = ["mcp", "--board", "/absolute/path/to/your/repo"]

# Optional: environment variables（ネストしたテーブルでも、inline tableでもOK）
[mcp_servers.kanban.env]
KANBAN_LOG = "info"
```

- ヒント（Codexの“癖”）:
  - TOMLは `mcp_servers`、JSONは `mcpServers` です。
  - `command` は実行ファイル、`args` に残りの引数（`mcp --board ...`）を与えます。
  - 必要に応じて `[mcp_servers.<name>.env]` で環境変数を渡せます。
  - 設定後、CodexのTUIで `tools/list` / `resources/list` が見えることを確認します。

### ツール名と応答フォーマット（重要）
- ツール名は常にフラット名（例: `kanban_new`, `kanban_list`, `kanban_relations_set`）です。
- `tools/call` の成功応答は、MCP仕様に従い `result.content[]`（`type:"text"`）で返します。本文はJSON文字列（例: `{"items":[...],"nextOffset":1}`）です。

### E2E（ローカル）
前提: codex CLI/jq が利用可能、`~/.codex/config.toml` に `mcp_servers.kanban` を設定済み。
```
cargo make e2e
```
BOARD/TIMEOUT は環境変数で上書きできます。

### ツール名（フラット名）
ツール名は常にフラットな形式（`^[a-zA-Z0-9_-]+$`）です。例: `kanban_new`, `kanban_list`, `kanban_relations_set`, `kanban_notes_append`。
`tools/call` も同じ名前で呼び出します。

## Claude Code への導入（MCP Server）
Claude Code（VS Code拡張）やClaude DesktopはMCPサーバを外部プロセスとして起動できる。代表的な設定例を示す。

### Claude Desktop（macOS/Windows）
`~/.claude/claude_desktop_config.json` に追記:

```json
{
  "mcpServers": {
    "kanban": {
      "command": "/absolute/path/to/kanban",
      "args": ["mcp", "--board", "/absolute/path/to/your/repo"],
      "env": {}
    }
  }
}
```

### Claude Code（VS Code 拡張）
`settings.json`（ユーザー/ワークスペース）に追記:

```json
{
  "claude.mcpServers": {
    "kanban": {
      "command": "/absolute/path/to/kanban",
      "args": ["mcp", "--board", "${workspaceFolder}"],
      "env": {}
    }
  }
}
```

- ヒント:
  - Windowsでは `command` は `C:\\path\\to\\kanban.exe` のように拡張子付きで指定します。
  - `--board` に `${workspaceFolder}` を与えると、VS Code のワークスペース直下の `.kanban/` を使えます。
  - 導入後、Claudeの「ツール/リソース」一覧で `kanban` が見えれば成功です。


## CIでのLint導入例（GitHub Actions）
`.github/workflows/ci.yml` のジョブ例です。
```yaml
name: CI
on: [push, pull_request]
jobs:
  lint-build-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
          profile: minimal
          override: true
      - name: Build & Test
        run: |
          cargo build --workspace --all-targets --locked
          cargo test  --workspace --all-targets --no-fail-fast
      - name: Lint Board (JSON, fail on error)
        run: |
          kanban lint --board . --json --fail-on error | tee lint.json
      - name: Upload Lint Artifact
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: lint-json
          path: lint.json
```

## 参考ドキュメント
- `docs/03_mcp_design.md`: MCPサーバの設計と起動
- `docs/04_api_spec.md`: ツールI/O（relations.setのワイルドカード、エラー例含む）
- `docs/CLI.md`: `kanban mcp|lint|reindex|compact` の使い方
- `docs/06_sequences.md`: 代表シーケンス（relations.set/Watchフラッシュ）
- `docs/CONTINUE.md`: 引継ぎ用の最短導線
 - `docs/codex_mcp_integration_notes.md`: Codex CLI（OpenAI製）とのMCP連携ノート（設定例/既知事象/ワークアラウンド）

### MCP応答形式（CallToolResult）ガイドライン（重要）
- `tools/call` の成功応答は、MCP仕様に従い `result.content[]` を必ず含めます。
- Codex CLI の `mcp-types` は `content[]` を `text|image|audio|resource*` の厳密型でデコードします。
- 互換性のため、`type:"text"` のブロック1件に、実データ（JSON）を「文字列化」して入れます（例: `text: "{\"items\":...}"`）。
- 既存クライアントとの後方互換のため、従来キー（`items` 等）も `result` のルートに残す設計としています。
