---
title: CLI/サブコマンド仕様
---

# CLI/サブコマンド仕様

本ドキュメントは、`kanban-mcp`が提供する単一バイナリ`kanban`のサブコマンド仕様です。MCPサーバ起動も運用系の`lint/reindex/compact`も、すべて同一バイナリのサブコマンドとして提供します。

## 共通
- ルート指定: `--board <PATH>` で`.kanban/`を含むボードのパスを明示します。
- 出力: 正常終了時は0、異常終了時は非0を返します。

## kanban mcp
- 目的: MCPサーバとして起動します（stdioベース）。
- 使用例:
  - `kanban mcp --board .`
  - `kanban mcp --openai --board .`（OpenAI互換ツール名で公開します）
- オプション（案）:
  - `--stdio` 既定（明示不要）
  - `--log-level trace|debug|info|warn|error` 既定: `info`
  - `--openai` OpenAI互換のツール名（`kanban_new`等）で `tools/list` を返します。
  - （将来）`--roots <PATHS>`: 現状は`--board`のみです。将来、複数rootsを受け付ける予定です。
- 動作:
  - JSON-RPC 2.0（stdio）で`tools/list`/`tools/call`/`resources/list`等を処理します。
  - MCP内で提供するツールは最小コア（`new/update/move/done/list/tree/watch/relations.set`）のみです。

### ツール名のモード切替の詳細
- 既定（標準モード）: `kanban/new`, `kanban/relations.set` のように、概念を表す階層/名前空間を保持した名前を返します。
- OpenAI互換（`--openai`）: `/` と `.` をアンダースコアに変換し、`kanban_new`, `kanban_relations_set` のように返します。
- 互換性: `tools/call` は両方の名前を受け付けます（サーバ内部で正規化しています）。

## kanban lint
- 目的: カード/関係の静的検査を実行します。
- 使用例:
  - `kanban lint --board .`
- オプション（案）:
  - `--fail-on error|warn` 既定: `error`（`warn`までを失敗扱いにするなら`warn`）
  - `--json` JSON出力にする
- 出力（人間向け）：
  - `WARN relations: dangling depends: 01ABC -> 01MISSING`
  - `ERROR parent_done: parent done but child not complete: 01PARENT`
  - 既定の分類: `missing*/dangling*/cycle`はERROR、`wip exceeded/self*/parent_done`はWARNです。
 - 退出コード（重要）：
   - 既定（`--fail-on error`）: ERRORが1件以上あれば`exit 1`、それ以外は`exit 0`。
   - `--fail-on warn`: WARN/ERRORを1件でも検出すれば`exit 1`。
 - 出力（JSON例, `--json` 有効時）:
```json
[
  {"severity":"warn","message":"wip exceeded: doing limit 3 actual 4"},
  {"severity":"error","message":"dangling depends: 01A -> 01MISSING"}
]
```

## kanban reindex
- 目的: `.kanban/cards.ndjson` と `.kanban/relations.ndjson` を再生成します。
- 使用例:
  - `kanban reindex --board .`
- オプション（案）:
  - `--cards-only` / `--relations-only`
  - `--full-scan`（既定）
- 出力（JSON例）:
  - `{ "duration_ms": 1234, "errors": [] }`

## kanban compact
- 目的: `done/YYYY/MM/` 等のパーティション整理や空ディレクトリ削除を行います（安全な範囲）。
- 使用例:
  - `kanban compact --board .`
- オプション（案）:
  - `--dry-run` 変更差分を表示のみ
  - `--remove-empty-dirs` 空ディレクトリ削除（既定ON）
- 仕様（最小）：
  - `done/`直下に残る`.md`を`done/YYYY/MM/`へ移動（`completed_at`の年月、無ければ保守値）。
  - その後、空ディレクトリを削除（指定時）。

## 実装メモ（後続）
- 単一バイナリ`kanban`（`kanban-mcp`クレートのbin）で`mcp/lint/reindex/compact`を提供します。
- MCP APIには`lint/reindex/compact`は含めず、あくまでローカル/CI運用のCLIとして提供します。
- 内部では共通ライブラリ（storage/index/lint）を再利用し、重複実装を避けます。

## kanban notes

### 追記（append）
- 目的: 作業ジャーナルに1件追記します（非冪等）。
- 例:
```
kanban notes-append --board . --card-id 01ABC... --text "Investigated parser error." --type worklog --tags investigation,parser --author alice
```
- 出力（JSON）: `{ "appended": true, "ts": "..." }`
 - 例（ファイルから本文を読み込み）:
```
kanban notes-append --board . --card-id 01ABC... --from-file ./note.md --type resume
```

### 取得（list）
- 目的: 最新N件（既定3）または全件を取得します。
- 例（最新3件）:
```
kanban notes-list --board . --card-id 01ABC... --limit 3
```
- 例（全件/JSON）:
```
kanban notes-list --board . --card-id 01ABC... --all --json
```
- 例（期間フィルタ since）:
```
kanban notes-list --board . --card-id 01ABC... --since 2025-09-05T00:00:00Z
```

## kanban update-fm
- 目的: カードFMの再開用フィールド（resume_hint/next_steps/blockers）を更新します。
- 例（resume_hintのみ更新）:
```
kanban update-fm --board . --card-id 01ABC... --resume-hint "Continue from section X; re-run tests Y"
```
- 例（next_stepsとblockersを複数指定）:
```
kanban update-fm --board . --card-id 01ABC... \
  --next "Refactor parser" --next "Update tests" \
  --blocker "Upstream lib issue"
```
