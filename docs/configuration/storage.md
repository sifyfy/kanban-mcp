---
title: ストレージ構成と設定
---

# ストレージ構成と設定

## ディレクトリ/ファイル配置（互換）
```
.kanban/
  columns.toml
  backlog/ todo/ doing/ review/ blocked/ done/
  templates/card.md
  generated/board.md
  cards.ndjson
```

## columns.toml（例）
```toml
[[columns]]
key = "backlog"; title = "Backlog"; wip_limit = 0
[[columns]]
key = "doing";   title = "Doing";   wip_limit = 2
[[columns]]
key = "review";  title = "Review";  wip_limit = 2
```

## カードファイル
- ファイル名: `<ULID>__<slug>.md`
- FMの`status`は参考値。真実は置かれているディレクトリ。

## ID採番
- ULID（モノトニック）を採用します。外部ロック不要で高い一意性と時系列ソート性を持ちます。

## サーバー設定（例: `kanban-mcp.config.yaml`）
```yaml
id:
  format: ulid
  short_length: 8      # UI表示向け、省略IDの桁数
  monotonic: true      # 同一プロセス内の順序安定化
filename:
  slug: true
lint:
  required_fields: ["id","title","lane","priority","size"]
  wip_enforce: warn   # warn|error
  parent_done_policy: warn   # enforce|warn|ignore
  relation_cycle: error      # error|warn
render:
  template: ".kanban/templates/board.md.hbs"
watch:
  enabled: true
writer:
  atomic_rename: true
  tmp_suffix: ".tmp"
list:
  default_limit: 200
  default_include_done: false
scan:
  hot_columns: ["backlog","todo","doing","review","blocked"]
  cold_columns: ["done"]
  include_done_default: false
  max_files_per_pass: 5000
  spot_check_percent: 2
  rebuild_threshold_percent: 10

index:
  enabled: true
  format: ndjson
  path: .kanban/cards.ndjson
relations_index:
  enabled: true
  path: .kanban/relations.ndjson

done:
  partition: yyyy-mm   # none | yyyy-mm | yyyy-q
  retention_months: 6
```

## 運用（Ops）メモ: パック化
- 古い`done`月を`.kanban/packed/`に圧縮する処理はMCPサーバーの管轄外（外部スクリプト/CI）です。
- 実施後はCLIの`kanban reindex`を実行してインデックスを整合化してください。

## 環境変数
- `KANBAN_MCP_LOG`（`info|debug`）
- `KANBAN_MCP_WATCH`（`0|1`、既定=1）
- `KANBAN_MCP_INDEX`（`ndjson`）



### Done格納ポリシー
- `done/`配下のディレクトリ分割は「完了日（completed_at）」に基づきます（作成日ではありません）。
- 例: 2025年9月に完了 → `done/2025/09/<ULID>__<slug>.md`。

## watch設定（columns.tomlの任意セクション）
```toml
[watch]
# 部分スキャン対象列（未指定時は columns、さらに無ければ ["backlog","doing"]）
hot_columns = ["backlog", "doing"]
# 通知のデバウンス間隔（ミリ秒）
debounce_ms = 300
# 1バッチの最大カード通知数
max_batch   = 50
```


## writer設定（columns.tomlの任意セクション）
```toml
[writer]
# 競合時に自動で別名にする（既定: false）
auto_rename_on_conflict = true
# 付与するサフィックス（-1 のように先頭の - は任意）
rename_suffix = "-dup"
```


## render設定（columns.tomlの任意セクション）
```toml
[render]
# 自動レンダ（board.md）の有効化（既定: false）
enabled = true
# レンダ用の専用デバウンス（ミリ秒）。
debounce_ms = 800
# 任意テンプレート: .kanban/templates/board.hbs or board.md.hbs
# 親進捗を別ファイルに出力（任意）します。単一または複数を指定できます。
# どちらか一方、`progress_parents` があれば優先されます。
# 生成物: .kanban/generated/progress_<ULID>.md と progress_index.md
progress_parent = "01PPPPPPPPPPPPPPPPPPPPPPPP"
progress_parents = ["01PPPPPPPPPPPPPPPPPPPPPPPP", "01QQQQQQQQQQQQQQQQQQQQQQQQ"]
```

### テンプレート・コンテキスト
- `columns[]`: `{ key, count }`
- `done`: done配下の合計件数
- `nonDone`: 非done列（columns配列）の合計件数
- `total`: 全件数（done + nonDone）
- `doneRate`: 完了率（0..1）
