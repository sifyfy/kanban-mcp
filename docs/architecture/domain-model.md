---
title: ドメインモデル定義
---

# ドメインモデル定義

## Board（ボード）
- 識別子: `boardId`（`roots`内パスから安定ハッシュ化）
- 物理: プロジェクトルート直下の`.kanban/`ディレクトリ
- 属性: name（任意）, path, columns, lanes, templates

## Column（列）
- key/title/wipLimit（`columns.toml`より）
- 実体: `.kanban/{key}/` ディレクトリ

## Card（カード）
- 実体: 列ディレクトリ配下のMarkdownファイル
- ファイル名: `<ULID>__<slug>.md` を推奨（`<slug>`は任意、人間可読の短い要約）。
- フロントマター（必須/推奨）
  - 必須: `id`, `title`, `lane`, `priority`, `size`
  - 推奨: `assignees[]`, `labels[]`, `created_at`, `depends_on[]`
  - 参考: `status`（真実は列ディレクトリ）

## Lane（レーン）
- ボード内の論理的なサブ流れ（例: `world-rs`）。`lanes/`配下のメタ定義は任意。

## Aggregate（集計）
- `.kanban/generated/board.md`。列別/レーン別/担当者別などのビューを生成。


## IDポリシー
- 形式: ULID（26文字、Crockford Base32、大文字のみ）。例: `01JB6M7Z3V6J7K2RX6H7M3H4Q9`。
- 特性: 時系列で辞書順ソートが可能。高い一意性と生成の分散性（ロック不要）。
- 生成: モノトニックULIDを採用（同一プロセス内の同時刻生成でも順序安定）。
- 人間向け短縮表示: `shortId`（末尾8文字など）をUI表示用に提供（衝突可能性があるため永続キーには使用しない）。
## Relations（親子・依存・関連）
- 親子: 子カードが`parent: <ULID>`で親を参照（親は`children`を持たない）。
- 依存: `depends_on: ULID[]`（DAG）。
- 関連: `relates_to: ULID[]`（任意の弱い関連）。

### 制約
- 親子は木構造（親は0/1、循環禁止）。
- 依存はDAG（循環検出時はlintで`error|warn`）。
- 親の完了: 既定で全子がdoneのときのみ親をdoneにできる（設定）。
