---
title: MCPサーバー設計
---

# MCPサーバー設計

本サーバーは Model Context Protocol (MCP) に準拠し、Resources/Tools/Prompts/Roots/Notifications を提供します。MCPはJSON-RPC 2.0ベースで、LLMクライアントがツール実行やリソース取得を行えます。

## トランスポート
- 既定: stdio（`kanban mcp`で起動）です。
- オプション: 開発時のみHTTP/WebSocketブリッジを用意します（本番はstdio推奨です）。

## 起動方法（CLI統合）
- サーバ起動: `kanban mcp --board <PATH>` です。
- ルート指定: 現状は`--board`のみ対応です（将来`--roots`を追加予定です）。
- ログ: `--log-level info`が既定です。CIでは`warn`以上を推奨します。

## ルート制限（Roots）
- クライアント提供の`roots`を採用し、探索対象を限定します。
- 各root配下をスキャンし、`.kanban/`を検出→ボードとして登録します。

## リソース（Resources）
- URI設計（仮想スキーム）
  - `kanban://{boardId}/board` → 集計ビュー（`generated/board.md`、将来は自動レンダ）
  - `kanban://{boardId}/columns` → `columns.toml`（TOMLテキスト）
  - `kanban://{boardId}/cards/{cardId}` → 単一カード（Markdown）
- Resource Templates（引数補完）
  - `kanban://{boardId}/cards/{cardId}` の`{boardId}`は候補提示、`{cardId}`はインデックス/検索で補完。
- Subscribe/通知
  - `.kanban/`配下のFS変更を監視し、対象URIに更新通知を発行。

## ツール（Tools / 最小コア）
代表のみ。詳細I/Oは [ツール/API仕様](../api/api-spec.md) を参照。
- `kanban/new`（カード新規）
- `kanban/move`（列移動）
- `kanban/done`（完了移動と`completed_at`付与）
- `kanban/update`（FM/本文の部分更新）
- `kanban/list`（絞込一覧）
- `kanban/tree`（木構造の取得）
- `kanban/watch`（更新通知の購読）
- `kanban/relations.set`（parent/depends/relatesの原子的適用）

## バックエンド構成
- file-backend（唯一）: 純Rustで実装（`std::fs`/`tokio::fs`）。外部の`cargo xtask`や`cargo make`への委譲は行いません。

### インデックス層
- 形式: ndjson（既定）。ファイル: `.kanban/cards.ndjson`。
- 更新: ツール実行時に行を追記/置換。クラッシュ時は原子的renameで復旧可能。
- 再構築: CLIサブコマンドでフルスキャン再生成（MCPでは提供しない）。

## エラーハンドリング
- 入力検証エラー: `invalid-argument`。
- パス越境/権限: `permission-denied`。
- 競合（ID重複/同時更新）: `conflict`。
- 不明: `internal`。

## 監視/通知（watch）
- `notify`で`.kanban/`を監視し、変更イベントを`notifications/publish`で標準出力へ通知（最小）。デバウンス(300ms)とまとめ通知（board + 変更cardのURI群）を実装。
- 通知形式: {"jsonrpc":"2.0","method":"notifications/publish","params":{"event":"resource/updated","uri":"kanban://{board}/board"}}
- 変更ファイル名から`<ULID>__`を抽出できた場合は `kanban://{board}/cards/{ULID}` への通知も送る。
- 監視溢れ(overflow)/エラー時は`board`更新のみ通知し、クライアント側の再取得を促す（将来は部分フルスキャン導入）。

## 例（JSON-RPC over stdio）
- 要求（tools/call）
```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"kanban/new","arguments":{"board":"main","title":"タスク","column":"backlog","lane":"core","priority":"P2","size":1}}}
```
- 応答
```json
{"jsonrpc":"2.0","id":1,"result":{"cardId":"01JB6...","path":".kanban/backlog/01JB6__タスク.md"}}
```
- エラー（方針）
```json
{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"invalid-argument","data":{"detail":"missing title"}}}
```
```
- エラー分類: invalid-argument / not-found / permission-denied / conflict / internal
## エラーポリシー（整形とフォールバックの扱い）
- tools/callは `invalid-argument|not-found|conflict|internal` に正規化し、`error.data.detail` に理由を格納する。
- relations差分更新は失敗時に `reindex_relations` へフォールバックする。フォールバックが発生した場合は、ツール結果（link/unlink）の `warnings[]` に `"relations: incremental update failed; ran full reindex"` を格納する。
## 通知の状態（watchとoverflow）
- `notify` イベントを300ms（既定。`columns.toml` の `[watch].debounce_ms` で変更可）でデバウンス。
- overflow（`paths==[]`）の場合は、`[watch].hot_columns`（なければ `columns`、さらに無ければ `backlog/doing`）を部分スキャンして不足を補う。
- overflowが連続3回以上続いた場合は、負荷抑止のため一度ボードのみの通知に切り替え、バースト終了後に通常モードへ戻す。
```

## 状態図: watch/overflow（テキスト図）
```
 [idle]
   | tools/call kanban/watch
   v
 [watching] -- fs event(paths!=[]) --> {buffer ids} --(debounce)--> notify(board+cards)
   |\
   | \__ fs event(paths==[]) [overflow] --> rescan(hot_columns) --> {buffer}
   |                                      --(>=3連続)--> notify(board) + clear
   |-- error --> rescan(hot_columns) --> notify(board+cards)
```

### 自動レンダ（将来）
- サーバー側でwatchイベントをトリガに`generated/board.md`を自動生成するワーカーを組み込む計画です（設定でON/OFF、デバウンス、原子的置換、通知発火）。


### フラッシュ処理の抽出（開発メモ）
- どのように: `tool_watch`内部の通知+自動レンダ処理は、小関数`do_watch_flush(board, base_uri, ids, last, last_render_out)`に抽出しました。
- なぜ: 本番/テストの双方で同一ロジックを利用し、将来の変更時にユニットで退行を検知しやすくするためです。
- 何を: 自動レンダ（テンプレ優先/シンプルフォールバック、原子的rename）とboard/cardsの通知を行い、デバウンス間隔と最終レンダ時刻を更新します。
