---
type: issue
id: ISSUE-002
title: Rust CLI Implementation
status: open
---

# Rust CLI Implementation

## 概要

Run/RunTemplate/Playlist を操作する Rust CLI を実装する。

## 要件

- RunTemplate から Run を生成し実行
- Playlist の管理・表示
- Run 履歴の管理
- 再現実行 (replay)
- Python module としても使用可能 (PyO3)
- nvim から subprocess で呼び出し可能

---

## アーキテクチャ

```
runbox (Rust)
├── CLI (clap)
├── Python module (PyO3)
└── Core logic (shared)

呼び出し元:
├── Terminal: runbox run ...
├── Python:   import runbox
└── nvim:     vim.fn.jobstart({"runbox", ...})
```

---

## CLI コマンド設計

```bash
# テンプレートから実行
runbox run --template <template_id> [--binding key=value]...

# テンプレート管理
runbox template list
runbox template show <template_id>
runbox template create <path>
runbox template delete <template_id>

# プレイリスト管理
runbox playlist list
runbox playlist show <playlist_id>
runbox playlist create <path>
runbox playlist add <playlist_id> <template_id>
runbox playlist remove <playlist_id> <template_id>

# 履歴
runbox history [--limit N]
runbox show <run_id>

# 再現実行
runbox replay <run_id>

# バリデーション
runbox validate <path>
```

---

## Crate 構成

```
crates/
├── runbox-core/    # Core types + logic
├── runbox-cli/     # CLI
└── runbox-py/      # PyO3 bindings
```

---

## 依存 crate

- clap: CLI parsing
- serde/serde_json: JSON handling
- uuid: ID generation
- git2: Git operations
- dirs: XDG directories
- dialoguer: Interactive prompts
- pyo3: Python bindings

---

## Acceptance Criteria

- [ ] `runbox run --template <id>` で RunTemplate から Run 生成 & 実行
- [ ] bindings の解決 (defaults, provided, interactive)
- [ ] git context capture (HEAD, diff)
- [ ] patch push to refs/patches/
- [ ] Run JSON の保存 (~/.local/share/runbox/runs/)
- [ ] `runbox replay <run_id>` で再現実行
- [ ] `runbox template list/show/create`
- [ ] `runbox playlist list/show`
- [ ] JSON Schema バリデーション
- [ ] PyO3 module として import 可能

---

## 関連

- ISSUE-001: Run/RunTemplate/Playlist Spec
