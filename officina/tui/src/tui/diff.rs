//! Line diff for tool blocks (2026-11-14).
//!
//! pi's edit tool returns a canonical unified diff in `result.details.diff`;
//! while the call is still running (and for `write` overwrites, where pi has
//! no diff) the TUI derives its own. Pure + bounded: inputs over 1200 lines
//! fall back to a size note instead of an O(n²) table.

/// Rows of a unified diff body (headers included first two).
pub fn unified(old: &str, new: &str, path: &str, context: usize, max_lines: usize) -> Vec<String> {
    if old == new {
        return Vec::new();
    }
    let a: Vec<&str> = old.split('\n').collect();
    let b: Vec<&str> = new.split('\n').collect();
    if a.len() > 1200 || b.len() > 1200 {
        return vec![
            format!("--- {path}"),
            format!("+++ {path}"),
            format!("(file too large to diff: {} → {} lines)", a.len(), b.len()),
        ];
    }
    let n = a.len();
    let m = b.len();
    let mut dp = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if a[i] == b[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut rows: Vec<(char, &str)> = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    while i < n && j < m {
        if a[i] == b[j] {
            rows.push((' ', a[i]));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            rows.push(('-', a[i]));
            i += 1;
        } else {
            rows.push(('+', b[j]));
            j += 1;
        }
    }
    while i < n {
        rows.push(('-', a[i]));
        i += 1;
    }
    while j < m {
        rows.push(('+', b[j]));
        j += 1;
    }

    let mut keep = vec![false; rows.len()];
    for (idx, (kind, _)) in rows.iter().enumerate() {
        if *kind == ' ' {
            continue;
        }
        let lo = idx.saturating_sub(context);
        let hi = (idx + context).min(rows.len().saturating_sub(1));
        for slot in keep.iter_mut().take(hi + 1).skip(lo) {
            *slot = true;
        }
    }

    let mut out: Vec<String> = vec![format!("--- {path}"), format!("+++ {path}")];
    let mut gap = false;
    for (idx, (kind, text)) in rows.iter().enumerate() {
        if out.len() >= max_lines {
            out.push("… [diff truncated]".to_string());
            break;
        }
        if !keep[idx] {
            gap = true;
            continue;
        }
        if gap && out.len() > 2 {
            out.push("@@".to_string());
        }
        gap = false;
        out.push(format!("{kind}{text}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_is_empty() {
        assert!(unified("a\nb", "a\nb", "x.rs", 2, 80).is_empty());
    }

    #[test]
    fn marks_changes_with_context() {
        let d = unified("a\nb\nc\n", "a\nB\nc\n", "x.rs", 1, 80);
        assert_eq!(d[0], "--- x.rs");
        assert!(d.contains(&"-b".to_string()));
        assert!(d.contains(&"+B".to_string()));
        assert!(d.contains(&" a".to_string()));
    }

    #[test]
    fn collapses_unchanged_gaps() {
        let old = (0..40)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut lines: Vec<String> = old.split('\n').map(String::from).collect();
        lines[5] = "changed-a".into();
        lines[35] = "changed-b".into();
        let d = unified(&old, &lines.join("\n"), "f.txt", 1, 80);
        assert!(d.iter().any(|l| l == "@@"));
        assert!(
            d.iter().all(|l| !l.starts_with(" l5")),
            "distant context dropped"
        );
    }

    #[test]
    fn oversized_inputs_fall_back() {
        let big = (0..1300)
            .map(|i| format!("l{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let d = unified(&big, &format!("{big}\nnew"), "big.txt", 2, 50);
        assert!(d[2].contains("file too large"));
    }

    #[test]
    fn respects_max_lines() {
        let old = (0..200)
            .map(|i| format!("a{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (0..200)
            .map(|i| format!("b{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let d = unified(&old, &new, "x", 1, 30);
        assert!(d.len() <= 31, "capped: {}", d.len());
        assert_eq!(d.last().unwrap(), "… [diff truncated]");
    }
}
