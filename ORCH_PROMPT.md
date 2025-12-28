<issue>

# Short ID Support

## 概要

git の短縮 SHA のように、ID を先頭数文字で指定できるようにする。

```bash
# 現状（長い）
runbox show run_550e8400-e29b-41d4-a716-446655440000

# 目標（短い）
runbox show 550e
```

---

## 設計

### ID 形式

現行の UUID 形式を維持し、**入力と表示で短縮を許可**:

| 種別 | フル形式 | 短縮形 |
|------|----------|--------|
| run_id | `run_550e8400-e29b-41d4-a716-...` | `550e8400` / `550e` |
| template_id | `tpl_a1b2c3d4-e5f6-...` | `a1b2c3d4` / `a1b2` |
| playlist_id | `pl_def45678-90ab-...` | `def45678` / `def4` |

### 解決ルール

1. **完全一致**を優先
2. 完全一致がなければ **prefix マッチ**
3. prefix マッチが複数ヒット → **エラー**（曖昧）
4. ヒットなし → **エラー**（not found）

```
入力: "550e"

run_550e8400-...  ← ヒット（1件なら採用）
run_550e9999-...  ← ヒット（2件以上なら曖昧エラー）
```

---

## 実装

### 1. ID 解決関数

```rust
/// ID を解決する（完全一致 or prefix マッチ）
pub fn resolve_id<T: HasId>(items: &[T], input: &str) -> Result<String> {
    // 完全一致
    if let Some(item) = items.iter().find(|i| i.id() == input) {
        return Ok(item.id().to_string());
    }

    // prefix 正規化（prefix を除去、ハイフン除去）
    let normalized = normalize_for_match(input);

    // prefix マッチ
    let matches: Vec<_> = items
        .iter()
        .filter(|i| {
            let id_normalized = normalize_for_match(i.id());
            id_normalized.starts_with(&normalized)
        })
        .collect();

    match matches.len() {
        0 => bail!("No item found matching '{}'", input),
        1 => Ok(matches[0].id().to_string()),
        n => bail!(
            "Ambiguous: {} items match '{}'. Use more characters.\n{}",
            n,
            input,
            matches.iter().map(|m| format!("  - {}", short_id(m.id()))).collect::<Vec<_>>().join("\n")
        ),
    }
}

/// ID を正規化（prefix除去、ハイフン除去、小文字化）
fn normalize_for_match(id: &str) -> String {
    id.trim_start_matches("run_")
      .trim_start_matches("tpl_")
      .trim_start_matches("pl_")
      .replace("-", "")
      .to_lowercase()
}

/// 表示用の短縮 ID（先頭8文字）
pub fn short_id(full_id: &str) -> String {
    let hex = normalize_for_match(full_id);
    hex.chars().take(8).collect()
}
```

### 2. 表示の短縮

```rust
// ps 出力例
println!("{:<10} {:<10} {:<8} {:<12} {}",
    short_id(&run.run_id),  // "550e8400"
    run.status,
    run.runtime,
    relative_time(run.timeline.started_at),
    truncate(&run.exec.argv.join(" "), 40),
);
```

```
ID        STATUS     RUNTIME  STARTED      COMMAND
550e8400  running    tmux     2 mins ago   python train.py --lr 0.001
a1b2c3d4  exited     bg       1 hour ago   ./build.sh
def45678  failed     tmux     3 hours ago  npm test
```

### 3. 既存コマンドの変更

| コマンド | 変更内容 |
|----------|----------|
| `runbox show <id>` | `resolve_id()` で解決 |
| `runbox replay <id>` | `resolve_id()` で解決 |
| `runbox history` | 表示を `short_id()` に |
| `runbox template show <id>` | `resolve_id()` で解決 |
| `runbox template list` | 表示を `short_id()` に |
| `runbox playlist show <id>` | `resolve_id()` で解決 |
| `runbox playlist list` | 表示を `short_id()` に |

### 4. 新規コマンド（ISSUE-005）の対応

| コマンド | 対応 |
|----------|------|
| `runbox ps` | 表示を `short_id()` に |
| `runbox logs <id>` | `resolve_id()` で解決 |
| `runbox attach <id>` | `resolve_id()` で解決 |
| `runbox stop <id>` | `resolve_id()` で解決 |

---

## UX

### 成功例

```bash
$ runbox ps
ID        STATUS   RUNTIME  COMMAND
550e8400  running  tmux     python train.py
a1b2c3d4  exited   bg       ./build.sh

$ runbox logs 550e
[showing logs for run_550e8400-e29b-41d4-...]

$ runbox attach 550
[attaching to tmux:session=runbox;window=run_550e8400]
```

### エラー例

```bash
$ runbox logs xyz
Error: No item found matching 'xyz'

$ runbox logs 5
Error: Ambiguous: 3 items match '5'. Use more characters.
  - 550e8400
  - 5a1b2c3d
  - 5def4567
```

### フル ID も引き続き使用可能

```bash
$ runbox show run_550e8400-e29b-41d4-a716-446655440000
# 従来通り動作
```

---

## Acceptance Criteria

- [ ] `resolve_id()` 関数を Storage に追加
- [ ] `short_id()` 関数を追加
- [ ] `runbox show` が短縮 ID を受け付ける
- [ ] `runbox replay` が短縮 ID を受け付ける
- [ ] `runbox history` が短縮 ID で表示
- [ ] `runbox template show/list` が短縮 ID 対応
- [ ] `runbox playlist show/list` が短縮 ID 対応
- [ ] 曖昧な入力時に候補を表示するエラーメッセージ
- [ ] ISSUE-005 の新規コマンドも短縮 ID 対応

---

## 関連

- ISSUE-002: Rust CLI Implementation（既存コマンド）
- ISSUE-005: Run Execution Management（新規コマンド）

</issue>

Instructions:
- Implement the changes described in the issue above
- Run tests to verify your changes work correctly
- When complete, create a pull request targeting `main`:
  - Title should summarize the change
  - Body should reference issue: ISSUE-006
  - Include a summary of changes made
