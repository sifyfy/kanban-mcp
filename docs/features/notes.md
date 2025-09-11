---
title: Notes（作業ジャーナル）仕様
---

# Notes（作業ジャーナル）仕様

目的（Why）
- カード本文に進捗メモを積み増すと肥大・ノイズ化するため、変動情報を外出しして管理します。
- LLMには既定で“最新N件のみ”を見せ、必要時だけ“全件”に広げます。

設計（What/How）
- 格納形式: NDJSON（1行=1ノート）。
  - パス: `.kanban/notes/<ULID>.ndjson`
  - 1行スキーマ: `{ ts: RFC3339, type: "worklog|resume|decision", text: string, tags?: string[], author?: string }`
- 参照単位: 最新N件（推奨N=3）または利用者の要望で全件です。サーバ側で要約や剪定は行いません。

FMの推奨フィールド（任意）
- `resume_hint: string` … 再開のための最小メモ（1–3文）
- `next_steps: string[]` … これからやること（〜5行）
- `blockers: string[]` … 阻害要因
- `last_note_at: string(RFC3339)` … 直近ノートのタイムスタンプ

MCPツール（I/O）
- `kanban/notes.append`（非冪等）
  - 入力: `{ board, cardId, text, type?, tags?, author? }`
  - 既定`type`: `worklog`
  - 出力: `{ appended: true, ts, path }`
- `kanban/notes.list`（読み取り/冪等）
  - 入力: `{ board, cardId, limit?, all? }`（既定 `limit=3`, `all=false`）
  - 出力: `{ items: NoteEntry[] }`（新しい順）

JSON-RPC例
```jsonc
// 進捗を1件追記
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"kanban/notes.append","arguments":{"board":".","cardId":"01ABC...","text":"Investigated parser error.","type":"worklog","tags":["investigation"]}}}

// 最新3件だけ取得
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"kanban/notes.list","arguments":{"board":".","cardId":"01ABC...","limit":3}}}

// 全件取得
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"kanban/notes.list","arguments":{"board":".","cardId":"01ABC...","all":true}}}
```

運用ガイド（LLM）
- 既定は`limit`で最新N件のみ読みます（推奨N=3）。必要時のみ`all:true`で全件へ拡張します。
- 重要な決定は`type:"decision"`にし、カード本文からリンクしてください（本文は短く維持します）。
- LLMが要約して`resume_hint/next_steps`に反映する場合は、`kanban/update`でFMを上書きします。

推奨サイズ（強制ではない）
- resume_hint: 1–3文
- next_steps: 〜5行
- 単一ノート: 短い段落（長大ログは分割）

アンチパターン（避けるべき）
- 本文に延々と進捗メモを追記し続ける（→ notes.append に外出し）
- `notes.list`で常に全件取得（→ まずは`limit`、必要時のみ全件）
- タイトル変更を頻繁に繰り返してノイズを増やす（→ まとめてupdate）
