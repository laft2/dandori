# Dandori

仕事専用の**個人用シングルタスク・キュー**。  
マルチタスクを避け、「今やること」を 1 つに絞って上から順に片付けるためのローカル完結タスク管理ツールです。

## 特徴

- **完全ローカル動作**：データは SQLite 1 ファイル（WAL モード）に保存し、外部サービスに依存しません。
- **AI（Claude Code 等）との協調**：
  - CLI `dan` から JSON 入出力で操作できます（`dan guide` で AI 向けガイドを出力）。
  - **人間の Undo（`⌘Z`）** と **AI のセッション単位ロールバック** を分離し、競合検出も備えています。
- **目に優しいライト UI**：オフホワイト背景（`#F6F5F1`）と高コントラストの文字色、文字サイズ変更（`⌘+` / `⌘-`）に対応。

## 構成

```
dandori/
├── crates/
│   ├── dandori-core/   # コアロジック（SQLite・Fractional Indexing・操作ログ・Undo/Rollback）
│   ├── dandori-cli/    # CLI バイナリ `dan`
│   └── dandori-app/    # Tauri v2 デスクトップアプリ
└── src/                # React + TypeScript + Tailwind CSS フロントエンド
```

## 開発セットアップ（mise）

```sh
mise trust
mise install
npm install
```

### よく使うコマンド

```sh
# Rust テスト一式を実行
mise run test

# CLI を使う
cargo run -q -p dandori-cli -- list
cargo run -q -p dandori-cli -- guide

# デスクトップアプリを起動（フロントエンド開発サーバー + Tauri）
npm run dev
# 別ターミナルで:
cargo run -p dandori-app
```

## 環境変数

| 変数名 | 説明 |
|---|---|
| `DANDORI_DB` | DB ファイルの保存先を上書きします（省略時は OS 標準のアプリデータフォルダ内 `Dandori/dandori.db`） |
| `DANDORI_SESSION` | CLI 操作を記録するセッション ID を明示指定します |

## CI / リリース（GitHub Actions）

- **CI**（`.github/workflows/ci.yml`）：すべての `push` および `main` への PR で macOS / Windows のテスト・`clippy`・`rustfmt`・フロントエンドビルドを実行します。
- **Release**（`.github/workflows/release.yml`）：`main` への `push` で [Release Please](https://github.com/googleapis/release-please) がリリース PR を自動作成し、マージすると macOS (Apple Silicon) / Windows (x64) 向けのデスクトップアプリと `dan` CLI をビルドして GitHub Releases に公開します。
  - GitHub リポジトリの `Settings → Actions → General → Workflow permissions` で **「Allow GitHub Actions to create and approve pull requests」** を有効にしてください。
  - macOS 版は未署名ビルドのため、初回起動前に以下を実行してください：
    ```sh
    xattr -cr /Applications/Dandori.app
    ```
