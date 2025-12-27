---
type: issue
id: ISSUE-001
title: Run/RunTemplate/Playlist Spec
status: open
---

# Run/RunTemplate/Playlist Spec

## 概要

- **Run**: 再現可能な実行記録
- **RunTemplate**: Run のテンプレート (bindings 未解決)
- **Playlist**: RunTemplate への参照リスト

## 詳細

See:
- [specs/run.md](../../specs/run.md)
- [specs/run.cue](../../specs/run.cue)

## Acceptance Criteria

- [ ] CUE schema が `cue vet` で検証済み
- [ ] JSON Schema の生成確認
- [ ] 実装 (ISSUE-002) での spec 変更をマージ
- [ ] Example JSON が schema に対して valid
