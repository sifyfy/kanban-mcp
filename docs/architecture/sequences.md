---
title: 代表ユースケースのシーケンス
---

# 代表ユースケースのシーケンス

以下はテキストベースの簡易シーケンス図です。

## 新規カード作成（kanban/new）
```
Client -> MCP Server: tools/call { name: "kanban/new", args }
MCP Server -> storage: nextUlid(); write card to .kanban/{column}/<ULID>__<slug>.md
MCP Server -> storage: update index (append or atomic rewrite)
MCP Server -> Client: result { cardId, uri, path }
MCP Server -> Client(s): resources/updated for kanban://{board}/board
```

## 列移動（kanban/move）
```
Client -> MCP Server: tools/call { name: "kanban/move", cardId, toColumn }
MCP Server -> storage: fs.rename(old, new)
MCP Server -> Client: result { from, to, path }
MCP Server -> Client(s): resources/updated for card + board
```


## 木構造取得（kanban/tree）
```
Client -> MCP Server: tools/call { name: "kanban/tree", root=<P>, depth=3 }
MCP Server -> relations-index: resolve children recursively
MCP Server -> index: join minimal card meta
MCP Server -> Client: nested tree JSON


## 関係を原子的に適用（kanban/relations.set）
```
Client -> MCP Server: tools/call { name: "kanban/relations.set", add[], remove[] }
MCP Server -> storage: apply FM changes (parent/depends/relates)
MCP Server -> relations-index: atomic rewrite with dedup & parent uniqueness
note right of MCP Server: parent一意検証（childごとに高々1本）
alt parent wildcard remove (to:"*")
  MCP Server -> storage: child.FM.parent = null
  MCP Server -> relations-index: remove all {type:"parent", from:child, to:*}
end
opt 差分更新失敗
  MCP Server -> relations-index: full reindex
  MCP Server -> Client: warnings[] += "relations: incremental update failed; ran full reindex"
end
MCP Server -> Client: { updated: true, warnings[] }
MCP Server -> Client(s): resources/updated for board (+ tree root if必要)
```

## Watchフラッシュ（do_watch_flush + WatchSink）
```
Watcher -> MCP Server: pending IDs batched
MCP Server -> storage: (optional) render board.md via template or simple renderer
MCP Server -> WatchSink: notifications/publish { uri: kanban://{board}/board }
loop for each id in IDs
  MCP Server -> WatchSink: notifications/publish { uri: kanban://{board}/cards/{id} }
end
```
備考:
- デバウンス: `watch.debounce_ms`（既定300ms）でまとめて出力します。
- overflow時: `hot_columns`（未指定なら`columns`→`[backlog,doing]`）を部分スキャンして補完します（上限`watch.max_batch`）。
