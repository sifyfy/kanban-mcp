---
title: 目的と全体像（kanban-mcp）
---

# 目的と全体像（kanban-mcp）

## 目的（Why）
- ファイルベースKanban（`.kanban/`配下に列ディレクトリとMarkdownカード）を、プロジェクト横断で同一UXのまま操作可能にするため、MCPサーバーとして外出しします。
- LLMクライアント（MCP対応）の「ツール」として、カード作成/移動/更新/集計/検査等を呼び出し可能にします。
- 既存の手運用やスクリプトに依存せず、単一のRustサーバーが直接ファイルを操作する方式に統一します。

## 非目的（Non-Goals）
- サードパーティSaaSの完全代替（通知/権限/課金/ダッシュボード等までは範囲外）。
- 複雑なワークフローエンジン。基本は「列=状態」「移動=状態遷移」に限定。

## 要求（What）
- `.kanban/`の既存構造を準拠:
  - `columns.toml`（列とWIP上限）
  - 列ディレクトリ: `backlog/ todo/ doing/ review/ blocked/ done/` 等
  - `templates/card.md` 雛形、`generated/board.md` 集計ビュー
- カードはMarkdown + YAMLフロントマター（id/title/lane/priority/size/assignees/labels/created_at/depends_on等）。
- ツール群（最低限）: new/move/done/update/list/tree/watch/relations.set
- WIP上限の検査、ID採番の一意性、スラッグ/ファイル名の正規化。
- クロスプラットフォーム: Windows/macOS/WSL/Linux。改行/パス差異の吸収。
- 監視: `.kanban/`配下の変更を検知し、MCPのリソース更新通知に反映。

## 方針（How）
- 実装言語: Rust。MCP（JSON‑RPC 2.0 over stdio）サーバーとして実装します。
- CLIエントリ: 単一バイナリ`kanban`で`mcp/lint/reindex/compact`を提供（運用系はCLI、MCP APIは最小コアに限定）。
- バックエンド: ファイル直接操作のみ（`cargo xtask`や`cargo make`等への委譲は行いません）。
- リソース表現: `kanban://{boardId}/...` の仮想URIで公開（実体は`file://`下の`.kanban`）。
- ルート制限: クライアント提供の`roots`に基づき、アクセス可能ディレクトリをホワイトリスト化。
- 書き込みは「一時ファイル→原子的rename」方式で耐障害性を確保。
- 主要コンポーネント（想定クレート）: `tokio`（非同期I/O）、`serde`/`serde_json`/`serde_yaml`（FM/JSON）、`toml`、`notify`（FS監視）、`handlebars`（レンダリング）、`slug`（スラッグ生成）。

## 全体構成（アーキテクチャ）
- server: MCPエントリ（stdio）。
- storage: `.kanban/`配下の列/カード/集計の読み書き。
- core:
  - id: 時系列ソート可能なULID採番（モノトニック）、重複検知。
  - columns: `columns.toml`のCRUDとWIP検査。
  - cards: FMの検証・編集・移動。
  - render: 将来はサーバー側ワーカーで`generated/board.md`を自動生成（テンプレートは任意）。
  - lint: 必須メタ/命名/WIP/依存の検査。
  - （運用）古いdone月のパック化はMCP APIではなく、外部スクリプトやCIによる運用手順として扱います。
  - watch: FS監視→MCP通知へ反映。
- api:
  - tools: new/move/done/update/list/tree/watch/relations.set
  - resources: board/columns/cardX等のテンプレート
  - prompts: 受入基準作成・分割支援など（任意）

## 既存運用の前提（From voco）
- 詳細は`voco/docs/kanban.md`参照。`.kanban`配下の列=ディレクトリ、1カード=1ファイル、`columns.toml`/WIP、`generated/board.md`生成等の流儀を踏襲します（ただし`cargo xtask`/`cargo make`依存は外し、本サーバーが全操作を直接実行します）。

## プラットフォーム対応（OS別ポイント）
- Windows: 既定で大文字小文字を区別しないFSを想定します。ファイル名の大文字小文字はサーバー側で正規化して扱います。
- macOS: FSEventsによる監視を利用します。Apple Silicon/Intelの両方でビルド可能です。既定のAPFS（大小文字非区別）を考慮し、IDやファイル名比較はケース非依存で行います。
- Linux: inotifyベースの監視を利用します。多量イベントに備えてバッファ溢れ時は全量再スキャンを行います。
- WSL: /mnt 上の監視はイベント欠落の可能性があるため、重要操作後に軽量スキャンで整合性を確認します。
