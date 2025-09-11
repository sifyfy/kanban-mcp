---
title: セキュリティと運用
---

# セキュリティと運用

## セキュリティ制約
- ルート制限: クライアントの`roots`に含まれないパスは読み書き不可。
- パス正規化: `..`やシンボリックリンク越境を拒否。
- 入力検証: 列キー/レーン/優先度/サイズ等の型と値域チェック。
- 原子的更新: 一時ファイル→renameで破損防止。
- ログ: 個人情報（assignees等）は必要最小限のマスキング。

## 運用
- ロギング: `info`（操作）/`debug`（詳細）/`error`（障害）。
- バックアップ: `.kanban/`配下を定期バックアップ（Git管理が基本）。
- 監視: FS監視スレッドの健全性、失敗時は自動再起動。
- CI: PRでCLIの`kanban lint`を実行し、将来は自動レンダ出力（board.md）を成果物化。

## プラットフォーム特性と対策
- ケース感度差: Windows/APFS(既定)は大小文字非区別、Linuxは区別。ID・ファイル名比較は常にケース非依存で扱い、出力時は大文字小文字の規約を固定（例: ULIDは大文字固定）。
- 監視基盤差: Windows=ReadDirectoryChangesW、macOS=FSEvents、Linux=inotify。イベントのまとめ送信や欠落に備え、重要操作（new/move/done/update）後は整合性スキャンをフォールバックで実施。
- 改行と文字コード: UTF-8を前提（FM/本文）。改行はLFに正規化して書き出し、既存CRLFは読み込み時に吸収。
- パス長と予約名: Windowsの予約名（CON/NUL等）やパス長制限に配慮し、スラッグ生成で安全なファイル名へ丸める。

### CIサンプル（GitHub Actions）
```yaml
name: kanban-lint
on:
  pull_request:
  push:
    branches: [ main ]
jobs:
  lint:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - name: Build kanban
        run: cargo build -q --workspace --bins
      - name: Run kanban lint (fail on error)
        run: target/debug/kanban lint --board . --json --fail-on error
```

### 依存・ライセンス監査テンプレ（cargo-deny/audit）
```yaml
name: security
on:
  pull_request:
  schedule:
    - cron: '0 3 * * *'  # 日次
jobs:
  deps-security:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: Install cargo-deny & audit
        run: |
          cargo install cargo-deny --locked || true
          cargo install cargo-audit --locked || true
      - name: cargo-deny (licenses/deps)
        run: cargo deny check --hide-inclusion-graph
      - name: cargo-audit (advisories)
        run: cargo audit -q
```

備考:
- 許可するライセンスは`deny.toml`の`[licenses].allow`で管理します（本リポは `MIT OR Apache-2.0` をワークスペース継承）。
- サプライチェーンの変化を捉えるため、PRトリガに加えてスケジュール実行（daily）を推奨します。


## CIパイプライン（推奨）
- 何を: build/testのOSマトリクスに加え、lint/format/依存チェックを自動化します。
- どのように: `.github/workflows/ci.yml` で以下ジョブを定義しました。
  - build-test: `cargo build/test --workspace --all-targets`（Linux/macOS/Windows）
  - lint: `cargo fmt --check` と `cargo clippy -D warnings`
  - security: `cargo deny check` と `cargo audit`（必要に応じて `deny.toml` で許可ライセンス/例外を管理）
- なぜ: 退行/警告/脆弱性を早期検知し、品質と運用安全性を高めるためです。
