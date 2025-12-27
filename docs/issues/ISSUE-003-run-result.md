---
type: issue
id: ISSUE-003
title: RunResult - Execution Result Recording
status: open
---

# RunResult - Execution Result Recording

## 概要

Run 実行後の結果を記録する RunResult の設計と実装。

## 要件

- 実行時刻、終了コード、duration の記録
- stdout/stderr の保存
- artifacts (出力ファイル) の保存

---

## Schema (案)

```cue
#RunResult: {
    result_id:   =~"^result_"
    run_id:      string

    execution: {
        started_at:  string  // ISO8601
        finished_at: string
        exit_code:   int
        duration_ms: int
    }

    output?: {
        stdout_ref?: string
        stderr_ref?: string
    }

    artifacts?: [...#Artifact]
}

#Artifact: {
    name: string
    path: string
    ref:  string
}
```

---

## 検討事項

- [ ] stdout/stderr の保存先 (inline vs blob store)
- [ ] artifacts の保存先
- [ ] 容量制限

---

## 関連

- ISSUE-001: Run Spec
- ISSUE-002: Rust CLI
