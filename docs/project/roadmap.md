---
title: ロードマップ
---

# ロードマップ

## フェーズ1（MVP: 最小コア）
- MCPツール: `new/update/move/done/list/tree/watch/relations.set` のみを提供（最小コア）。
- CLI統合: 単一バイナリ`kanban`に`mcp/lint/reindex/compact`を実装（`lint/reindex/compact`はMCP外の運用系）。
- Resources: board/columns/cardsの基本公開と更新通知（FS監視）。
- セキュリティ: `roots`制約と基本的な入力検証。

## フェーズ2（関係・レンダ強化）
- 関係: `relations.set`の運用強化、`kanban/tree`の最適化（ロールアップはクライアント計算）。
- 自動レンダ: watchトリガで`generated/board.md`を原子的に生成（テンプレート優先/フォールバック）。
- Lint強化: 親子循環/依存循環/参照切れ/親doneポリシーの詳細化（CLI）。

## フェーズ3（UX/Perf）
- Prompts整備（受入基準ドラフト、分割支援）。
- パフォーマンス最適化（インデックスの部分再構築・キャッシュ）。
- 変更サマリ生成や`stats`相当の計測はクライアント/CI側での集計を前提に検討。
