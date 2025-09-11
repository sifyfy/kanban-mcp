---
title: ツール/API仕様（I/O）
---

# ツール/API仕様（I/O）

以下はMCPツールの入出力仕様（JSONスキーマ相当の説明）です。成功時は`result`、失敗時はエラーコード+メッセージを返します。

## LLM Tool TL;DR (English)
- kanban/new: Create a new card. Non-idempotent (avoid duplicates). Required: board, title. Default column: backlog.
- kanban/move: Move a card to another column. Idempotent if already in the target column. Required: board, cardId, toColumn.
- kanban/done: Mark a card as done and move it to done/YYYY/MM/. Returns completed_at. Required: board, cardId.
- kanban/list: List cards with filters and pagination. Always pass columns to limit scope; prefer limit ≤ 200. query/includeDone may fall back to FS scanning. Required: board.
- kanban/tree: Return a parent-children tree rooted at an ID (read-only). Required: board, root. Optional: depth (default 3).
- kanban/watch: Start a filesystem watch and emit notifications/publish events (long-running; not for batch). Required: board.
- kanban/update: Update card front-matter and/or body. Title changes may rename the file per [writer] settings; warnings may appear. Required: board, cardId, patch.
- kanban/relations.set: Atomically apply add/remove of parent/depends/relates. At most one parent per child. Use to:"*" to clear an existing parent. Required: board.
- kanban/notes.append: Append a journal note to a card (worklog/resume/decision). Required: board, cardId, text.
- kanban/notes.list: List journal notes for a card. Default returns latest N (e.g., 3). Pass all:true for full history. Required: board, cardId.

## Resources (read-only)
- Manual: `resources/list` -> `kanban://{board}/manual` (Markdown)
- Card State: `resources/list {cardId}` -> `kanban://{board}/cards/{id}/state` (JSON)
  - Params for `resources/read`: `mode=brief|full` (default brief), `limit` (default 3)

Notes for LLMs:
- Prefer scoped queries (columns, small limits) to avoid expensive filesystem scans.
- Treat new as non-idempotent; move/done/update are safe to retry with same inputs.
- Surface warnings to users when present (e.g., auto-rename on title conflicts).

## 共通
- `board`: string（必須）…`boardId`。`roots`配下から検出したボード識別子。
- `cardId`: string（ULID。例: `01JB6M7Z3V6J7K2RX6H7M3H4Q9`）。

## kanban/new
- 入力
  - `board`（必須）
  - `title`（必須, string）
  - `column`（省略可, string, 既定=`backlog`）
  - `lane`（省略可, string）
  - `priority`（省略可, enum: `P0|P1|P2|P3`）
  - `size`（省略可, integer）
  - `labels`（省略可, string[]）
  - `assignees`（省略可, string[]）
  - `body`（省略可, string, Markdown）
- 出力
  - `cardId`, `path`
- 例（入力）:
```json
{"name":"kanban/new","arguments":{"board":".","title":"Spec","column":"backlog","labels":["doc"],"assignees":["alice"],"body":"Write spec first"}}
```

## kanban/move
- 入力: `board`, `cardId`, `toColumn`（必須）
- 出力: `from`, `to`, `path`（新パス）

## kanban/done
- 入力: `board`, `cardId`
- 出力: `completed_at`, `path`

## kanban/update
- 入力: `board`, `cardId`, `patch`
- writer: `columns.toml`の`[writer]`に`auto_rename_on_conflict`/`rename_suffix`がある場合、ファイル名の競合時に自動的に別名へリネーム（`warnings[]`に結果を記録）
- 備考: リネーム競合が発生した場合、`result.warnings[]`に理由を格納（例: "rename target exists; kept original filename"）
  - `patch.fm`（部分更新: lane/priority/size/assignees/labels/depends_on など）
    - 原則: 「未指定=無変更」。`[]` を指定した場合は空集合として上書き。
  - `patch.body`（オブジェクト）
    - 形式: `{ "text": string, "replace": boolean }`
    - `replace:false`（既定）: 本文末尾に追記。既存本文が非空かつ末尾改行が無ければ1つ改行を挿入してから `text` を追加し、最後に改行を1つ付ける。
    - `replace:true`         : 本文を `text` で置換（末尾改行は強制しない）。
    - バリデーション: `patch.body` がオブジェクトでない、または `text` 欠落、または `replace:true` かつ `text` 未指定は `invalid-argument`。
- 出力: `updated`（差分概要）

### 例: update（追記）
```json
{"name":"kanban/update","arguments":{"board":".","cardId":"01ABC...","patch":{"body":{"text":"append line","replace":false}}}}
```

### 例: update（置換）
```json
{"name":"kanban/update","arguments":{"board":".","cardId":"01ABC...","patch":{"fm":{"title":"New Title"},"body":{"text":"full body","replace":true}}}}
```

### 例: update（warnings付き）
```json
{"name":"kanban/update","arguments":{"board":"main","cardId":"01ABC...","patch":{"fm":{"title":"重複するタイトル"}}}}
```
- 出力例:
```json
{"updated":true,"column":"backlog","path":".kanban/backlog/01ABC__old-title.md","warnings":["rename target exists; kept original filename: .kanban/backlog/01ABC__new-title.md"]}
```

（注）カード本文の取得はファイル直読で代替可能です（MCPに`read`はありません）。

## kanban/list
- 入力: `board`, フィルタ
  - `columns`（string[]）/`column`（string, 非推奨）
  - `lane`, `assignee`, `label`, `priority`, `query`（タイトル/本文/IDの部分一致）
  - `includeDone`（bool, 既定=false）: `.kanban/done/`配下を含める
  - ページング: `offset`（既定0）, `limit`（既定200）
- 出力: `items[]`（`{cardId,title,column,lane}`）, `nextOffset`（存在すれば次オフセット）
- 例（入力）:
```json
{
  "name": "kanban/list",
  "arguments": {
    "board": "main",
    "column": "backlog",
    "columns": ["backlog","doing"],
    "lane": "core",
    "query": "最適化",
    "includeDone": false,
    "offset": 0,
    "limit": 50
  }
}
```
- 例（出力）:
```json
{
  "items": [
    {"cardId":"01JB6...","title":"FFT最適化","column":"doing","lane":"core"}
  ],
  "nextOffset": null
}
```

（注）レンダリングは将来、サーバー側バックグラウンドの自動生成ワーカーで対応予定です（APIは提供しません）。

（注）LintはCLIサブコマンドとして提供予定です（MCPには含みません）。


（注）reindexはCLIサブコマンドとして提供予定です（MCPには含みません）。

（注）compactはCLIサブコマンドとして提供予定です（MCPには含みません）。


（注）statsは必要に応じてクライアント側で`cards.ndjson`から算出してください（MCPには含みません）。

## エラーコード
- `invalid-argument`, `not-found`, `permission-denied`, `conflict`, `internal`

### エラー応答の例（load-bearing）
- `invalid-argument`（必須引数の欠落）
```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"invalid-argument","data":{"detail":"missing argument: root"}}}
```
- `not-found`（カード未検出）
```json
{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"not-found","data":{"detail":"card 01ZZ..."}}}
```
- `conflict`（parent一意違反 等）
```json
{"jsonrpc":"2.0","id":3,"error":{"code":-32000,"message":"conflict","data":{"detail":"multiple parent edges for child 01C..."}}}
```
- `internal`（I/O失敗 等）
```json
{"jsonrpc":"2.0","id":4,"error":{"code":-32000,"message":"internal","data":{"detail":"..."}}}
```

（splitは提供しません。`kanban/new`と`kanban/relations.set`の合成で代替してください。）
## kanban/relations.set
- 入力: `board`, `add[]`, `remove[]`
  - `add[]`: `[{"type":"parent|depends|relates","from":"ULID","to":"ULID"}]`
  - `remove[]`: `[{"type":"parent|depends|relates","from":"ULID","to":"ULID|*"}]`（parentは`to:*`で既存親を一括解除）
- 出力: `updated: true`, `warnings[]`（差分更新失敗→reindex実行時にメッセージを格納）

- 仕様（ワイルドカード）: `type:"parent"` かつ `to:"*"` の場合、`from`で指定した子の親エッジを全て削除します（結果、FMの`parent`は`null`になり、`relations.ndjson`上の該当行も消えます）。
- 整合性: `parent`は子`from`あたり高々1本となるように差分適用時に一意性を検証します。複数に分岐する入力が来た場合は`conflict`を返します。

- 正常時の追加情報: `warnings[]`（同上）
## kanban/tree（新規）
- 入力: `board`, `root`（ULID）, `depth`（int, 既定=3）
- 出力: `tree`（`{id,title,column,children:[...]}`）

（rollupは提供しません。treeの結果からクライアント側で計算してください。）



### 例: relations.set（parent一意・差分適用）
- 入力（子C→親Pを設定）:
```json
{"name":"kanban/relations.set","arguments":{"board":"main","from":"01CCHILD123456789ABCDEFGH","to":"01PPARENT123456789ABCDEFG","type":"parent"}}
```
- 差分適用の内容:
  - remove: `{type:"parent", from:"01CCHILD123456789ABCDEFGH", to:"*"}`（既存の親を全て除去）
  - add   : `{type:"parent", from:"01CCHILD123456789ABCDEFGH", to:"01PPARENT123456789ABCDEFG"}`
- 出力（成功）:
```json
{"updated": true, "warnings": []}
```
- 備考: 失敗時は`relations.ndjson`を全再構築し、`warnings[]`に`"relations: incremental update failed; ran full reindex"`を格納します。

### 例: relations.set（parentのワイルドカード削除）
```json
{
  "name": "kanban/relations.set",
  "arguments": {
    "board": "main",
    "remove": [
      {"type": "parent", "from": "01CCHILD...", "to": "*"}
    ]
  }
}
```
- 効果: `01CCHILD...` のFM上の`parent`が`null`になり、`relations.ndjson`の `{type:"parent", from:"01CCHILD...", to:"..."}` 行が削除されます。

- 備考: 返却に `warnings[]` が含まれることがある（差分更新失敗→reindex実行時）。
- 入力（親子）:
```json
{
  "name": "kanban/relations.set",
  "arguments": {"board":"main","from":"01CHILD...","to":"01PARENT...","type":"parent"}
}
```
- 入力（依存）:
```json
{"name":"kanban/relations.set","arguments":{"board":"main","from":"01A...","to":"01B...","type":"depends"}}
```
- 出力: `{ "updated": true }`

（splitの例は削除）
```json
{
（splitの例は削除）
  "arguments": {
    "board": "main",
    "column": "backlog",
    "parent": {"title":"音声合成高速化","lane":"core","priority":"P1","size":3},
    "children": [
      {"title":"プロファイル計測","size":1},
      {"title":"SIMD最適化","size":2}
    ]
  }
}
```
- 出力: `{ "parentId":"01P...","children":[{"id":"01C1...","path":"..."},{"id":"01C2...","path":"..."}], "warnings": [] }`

### 例: tree
```json
{"name":"kanban/tree","arguments":{"board":"main","root":"01P...","depth":3}}
```
- 出力（抜粋）: `{ "tree": {"id":"01P...","children":[{"id":"01C1..."}]}}`

（rollupの例は削除）
```json
{"name":"kanban/tree","arguments":{"board":"main","root":"01P...","mode":"count"}}
```
- 備考: 親子のprogress/ロールアップはクライアント側で計算（またはレンダ側で表示）する前提です（現行APIはprogressを返しません）。


## kanban/watch
- 入力: `board`
- 出力: `{ started: bool, alreadyWatching?: bool }`
- 備考: 通知は`notifications/publish`で標準出力へ出す（最小）。

- 設定（`.kanban/columns.toml` 任意）:
  - `[watch]`
    - `hot_columns`（string[]）…部分スキャン対象。未指定時は`columns`、それも無ければ`["backlog","doing"]`。
    - `debounce_ms`（u64）…通知デバウンス間隔（既定: 300）。
    - `max_batch`（usize）…一度にまとめるカード通知の上限（既定: 50）。

- 通知例:
```json
{"jsonrpc":"2.0","method":"notifications/publish","params":{"event":"resource/updated","uri":"kanban://./board"}}
{"jsonrpc":"2.0","method":"notifications/publish","params":{"event":"resource/updated","uri":"kanban://./cards/01ABCDEFGHJKLMNPQRSTVWXYZ"}}
```

- 実ログ例（複合イベント + overflow混在）:
```json
{"jsonrpc":"2.0","method":"notifications/publish","params":{"event":"resource/updated","uri":"kanban://./board"}}
{"jsonrpc":"2.0","method":"notifications/publish","params":{"event":"resource/updated","uri":"kanban://./cards/01HOTSLOTAAAAAAAAAAAAAAA"}}
{"jsonrpc":"2.0","method":"notifications/publish","params":{"event":"resource/updated","uri":"kanban://./cards/01HOTSLOTBBBBBBBBBBBBBBB"}}
```
（注）`paths==[]` のoverflowが3回続いた場合は、ボードのみの通知に切り替えた後、通常モードへ戻します。

- 例（フィルタ）:
```json
{"name":"kanban/list","arguments":{"board":"main","columns":["backlog"],"lane":"core","assignee":"alice","label":"x","priority":"P1","query":"banana","includeDone":true,"offset":0,"limit":50}}
```


## 注意: インデックスと通知の整合（最小）
- relationsの更新: `kanban/relations.set`は`.kanban/relations.ndjson`を差分更新し、同一三つ組の重複を排除してから原子的に置換します（tmp→rename）。親エッジは`child(from)`あたり高々1本になるよう一意検証します。失敗時は`reindex`へフォールバックします。
- watch overflow: `notify`イベントの`paths`が空の場合はoverflowとみなし、hot列（`watch.hot_columns`→`columns`→`[backlog,doing]`）を部分スキャンして`cards/{id}`をバッチ通知します（`watch.debounce_ms`間隔・`watch.max_batch`上限）。連続overflowが一定回数（3回）を超えた場合はボードのみの通知に切り替えて負荷を避けます。
