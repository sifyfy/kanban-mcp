---
title: kanban-mcp 設計資料 目次（再構成）
---

# kanban-mcp 設計資料 目次（再構成）

本ドキュメント群は、ファイルベースKanbanをMCPサーバーとして再利用可能にするための設計資料です。対象プラットフォームは Windows / macOS / Linux（WSL含む）です。

## はじめに
- 概要: getting-started/overview.md

## アーキテクチャ
- ドメインモデル: architecture/domain-model.md
- MCPサーバー設計: architecture/mcp-design.md
- 代表シーケンス: architecture/sequences.md
- 親子・依存関係: architecture/relations.md

## API・仕様
- ツール/API仕様: api/api-spec.md

## 設定
- ストレージ構成と設定: configuration/storage.md

## 運用
- セキュリティと運用: operations/security-ops.md
- 性能・スケーリング: operations/performance-scaling.md

## スタイル/ガイド
- Tool記述スタイル: style/tool-style.md

## 機能詳細
- Notes（作業ジャーナル）: features/notes.md

## 参考・統合
- CLI仕様: reference/cli.md
- 参考資料: reference/references.md
- Codex CLI連携ノート: integrations/codex-cli.md

## プロジェクト
- ロードマップ: project/roadmap.md

使用ヒント:
- MCPサーバ起動: `kanban mcp --board <PATH>`
- 運用系: `kanban lint|reindex|compact --board <PATH>`
