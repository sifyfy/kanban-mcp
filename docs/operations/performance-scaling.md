---
title: 性能・スケーリング設計（大規模done対策）
---

# 性能・スケーリング設計（大規模done対策）

## 背景（Why）
- `.kanban/done/` に大量のカードが溜まると、フルスキャン時のI/Oがボトルネックになります。
- 監視イベント（FS Watcher）が溢れた場合や整合性確認が必要な場合に、全列再スキャンが必要になることがあります。

## 方針（How）
1. ホット/コールド分離スキャン
   - 既定で`backlog/todo/doing/review/blocked`は「ホット列」とし、常時スキャン対象にします。
   - `done`は「コールド列」とし、フルスキャンの常時対象から外します。変更は監視イベントとインデックスで追跡します。

2. インデックス駆動（Append-Only Index）
   - `.kanban/cards.ndjson` にカードの最小メタ（`id,title,column,lane,assignees,labels,created_at,completed_at,updated_at,path`）を1行1レコードで追記管理します。
   - `new/move/done/update` 実行時にサーバーがインデックスへ追記更新（原子的書き換え）します。
   - 通常の`list/stats/render/lint`はインデックス経由で対象集合を決定し、必要時のみファイルを遅延読み込みします。
   - `.kanban/relations.ndjson` に関係エッジ（親子/依存/関連）を追記管理し、`tree/rollup/lint`を高速化します。

3. ディレクトリ分割（Sharding）
   - `done`直下に大量ファイルを置かない方針として、設定により`done/YYYY/MM/`または`done/YYYY/Qn/`へ自動分割します。
   - 古い月は任意でパック（`.kanban/packed/`）してワーキングツリーの点数を削減します。

4. パック化（Pack：運用）
   - パックはMCP APIではなく、外部スクリプト/CIで実施します。実施後はCLIの`kanban reindex`で整合性を回復します。
   - 実行基準（例）: 直近Nヶ月を除く月、または月内件数/容量がしきい値超過。

5. 部分検証（Spot Check）
   - 監視イベント喪失や異常終了後の再起動時は、`done`を全スキャンせず、N%のサンプリングで存在確認します。ズレ率が閾値を超えたときのみ対象サブディレクトリを限定フルスキャンします。

6. 遅延パース（Lazy Parsing）
   - 一覧・統計はFM最小集合のみで返却し、本文は要求時にのみ読み込みます。

7. 再索引（Reindex）
   - 明示的にCLIの`kanban reindex`ツールで`columns=[done]`等を指定し、対象のみフルリビルド可能。

## APIへの影響（What）
- `kanban/list`に以下の引数を追加します:
  - `includeDone`（bool, 既定=false）
  - `columns`（string[]）
  - `fromDays`（int）/`fromDate`（string, ISO8601）
  - `limit`（int, 既定=200）/`cursor`（string）
- 新ツール
  - CLIの`kanban reindex`: 対象列/範囲を指定してインデックスを再構築。
  - CLIの`kanban compact`: `done`の分割と空ディレクトリ整理（移動しない）。
  - （クライアント側で集計可能）stats: インデックスから高速に集計（列/レーン/担当/ラベル）。

## 設定（Config）
```yaml
scan:
  hot_columns: ["backlog","todo","doing","review","blocked"]
  cold_columns: ["done"]
  include_done_default: false
  max_files_per_pass: 5000
  spot_check_percent: 2
  rebuild_threshold_percent: 10

index:
  enabled: true
  format: ndjson         # ndjson | sqlite（将来拡張）
  path: .kanban/cards.ndjson

done:
  partition: yyyy-mm     # none | yyyy-mm | yyyy-q
```

## フルスキャン時の挙動（When）
- 通常起動: ホット列のみディレクトリ一覧→必要カードを遅延パース。
- 監視溢れ/異常検知: コールド列はサンプリング検証→閾値超過でサブディレクトリ単位のフルスキャン。
- 明示再索引: CLIの`kanban reindex`が対象をフルスキャンし、インデックスを置換します。

## 実装メモ
- ndjsonは1レコード1行で追記・読み出しが高速。置換時は一時ファイルへ全量書き出し→原子的rename。
- 文字比較はケース非依存（Windows/macOS考慮）。パスは正規形も合わせて保持。
- `partition=yyyy-mm`時は`done/2025/09/`のように保管し、一覧はインデックスから月別に絞ることでディレクトリエントリの爆発を防ぎます。
