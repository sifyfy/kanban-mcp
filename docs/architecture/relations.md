---
title: 親子・依存関係とLLM支援
---

# 親子・依存関係とLLM支援

## 目的（Why）
- 現実の開発では「親タスク（エピック/トラッキング）-子タスク（実作業）」構造や、順序制約のある「依存関係（depends_on）」が頻出します。
- LLMクライアントが容易に分割・リンク・進捗集約を扱えるよう、MCPツールとプロンプト、インデックス、レンダを設計します。

## モデル（What）
- 親子（Tree）: 階層の向きは「子が親を参照」。親は`children`を持たず、インデックスから導出します。
- 依存（DAG）: `depends_on: ULID[]`。循環は禁止（lintで検出）します。
- 関連（弱連結）: `relates_to: ULID[]`。意味は自由、循環検査対象外。

### フロントマター（FM）項目
```yaml
id: 01JB6M7Z3V6J7K2RX6H7M3H4Q9   # ULID
parent: 01JB6M7Z3V6J7K2RX6H7M3AAAA # 任意。親がない場合は省略
depends_on: []                   # 任意。順序制約（DAG）
relates_to: []                   # 任意。弱い関連
size: 2                          # 見積（ロールアップに使用可）
```

### 不変条件（Constraints）
- 親子は木構造（同一ボード内）。循環を禁止、1カードに親は0/1のみ。
- 依存はDAG。循環検出時は`lint`で`error|warn`（設定）とします。
- 親の`done`は、既定で「全子がdone」のときのみ許可（設定で`enforce|warn|ignore`）。

## インデックス（How）
- `.kanban/cards.ndjson` に最小メタ（id/title/column/lane/...）
- `.kanban/relations.ndjson` に関係エッジを保持（1行1エッジ、三つ組は大文字ULID）
  - `{"type":"parent","from":"<CHILD_ULID>","to":"<PARENT_ULID>"}`
  - `{"type":"depends","from":"<ULID>","to":"<ULID>"}`
  - `{"type":"relates","from":"<ULID>","to":"<ULID>"}`（必要に応じて双方向を2行）
- `link/unlink/update` で差分適用（重複排除＋原子的置換）。失敗時は `reindex_relations()` で全再構築。

## 進捗ロールアップ（Rollup）
- デフォルト: 子タスク個数ベース。`progress = done_children / total_children`
- オプション: `size`重み付き。`progress = sum(done.size) / sum(all.size)`
- 親に`progress`をFMへ書き戻さず、レンダやAPIが計算して返却（真実はインデックス）。

## MCPツール（Tools）
- `kanban/relations.set`: 親/依存/関連の追加・削除を原子的に適用。
- `kanban/tree`: 親（または任意カード）を根に木構造を返す。

### ワイルドカード削除（to:"*") のユースケース（親入れ替え）
- 目的: 既存の親を一旦すべて外してから、新しい親を割り当てたいケースです。
- 例:
  1) `remove: [{"type":"parent","from":"01CHILD...","to":"*"}]`
  2) `add:    [{"type":"parent","from":"01CHILD...","to":"01NEWPARENT..."}]`
- 効果: FMの`parent`は`null→01NEWPARENT...`へ更新され、`relations.ndjson`のparent行も差分更新されます。
- 注意: `parent`は子1つにつき高々1本です。複数親が発生する入力は`conflict`になります。

## プロンプト（Prompts）
- 例: 子タスク案を生成し、クライアント側で`kanban/new`と`kanban/relations.set`を合成して登録。

## レンダ（Resources）
- `kanban://{board}/tree/{cardId}`: 木構造のMarkdownビューを生成（`generated/tree_{cardId}.md`）。
- `kanban://{board}/board`: 既存ボードに親の進捗(%)を併記。

## Lint規則（Relations）
- 親子循環、依存循環、参照切れ、親done違反、子の孤児（`parent`が存在しない）を検出。
- 既定レベル: 親done違反=warn、循環=error、参照切れ=error。

## 例（Split入力）
```json
{
  "board": "main",
  "parent": {"title": "音声合成エンジンの高速化", "lane": "core", "priority": "P1"},
  "children": [
    {"title": "プロファイラ計測とボトルネック抽出", "size": 2, "labels": ["perf"]},
    {"title": "FFT実装のSIMD最適化", "size": 3, "labels": ["dsp","simd"]},
    {"title": "ユニット/負荷テスト整備", "size": 2, "labels": ["test"]}
  ]
}
```



## ポリシーと衝突解決
- 親doneポリシー: enforce|warn|ignore（設定: `parent_done_policy`）。enforce時は親をdoneにするAPIで未完の子が存在すればエラー。
- 依存循環: `lint_relations`で検出。CLIの`kanban lint`で検出し、必要に応じてCIで失敗扱いにできます。
- 親子循環: `lint_relations`で検出。`set_parent`は自身または子孫を親に設定する要求を拒否する（将来）。
- split時: 子には`parent`を自動付与。IDはULID、ファイル名は`<ULID>__<slug>.md`。
- relates: 双方向性は強制しない（必要なら両方向をリンク）。
