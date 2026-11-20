// File-size ratchet (2026-11-18).
//
// The TUI's three biggest files hold 63 % of the crate (state.rs 9.6k, mod.rs
// 5.8k, layout.rs 5.6k). Splitting them is a multi-stage job; what must not
// happen in the meantime is them GROWING. This test freezes the current size of
// every source file and fails in both directions:
//
//   - a file above its budget  -> new behaviour landed in a monolith;
//   - a file below its budget  -> the budget is stale and must be lowered, or the
//                                 ratchet silently re-opens the room that was won.
//
// When you genuinely must add lines to a budgeted file, raise its number in the
// same commit and say why in the message — that is the review moment.

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Frozen line counts, 2026-11-18. Lowering these over time is the point.
    const BUDGETS: &[(&str, usize)] = &[
        // +2 (2026-11-19): aethership rename in theme help + repaint test.
        ("src/tui/state.rs", 9_606),
        // +2 (2026-11-19): reasoning_guard registration and its one-line call in agent_end.
        ("src/tui/mod.rs", 5_784),
        // +8 (2026-11-19): /-menu full-surface PANEL fill (bg-leak fix).
        ("src/tui/layout.rs", 5_625),
        ("src/session_guard.rs", 1_324),
        ("src/tui/caduceus.rs", 1_253),
        // +12 (2026-11-19): monokai + catppuccin embedded-theme resolution.
        ("src/markdown.rs", 1_263),
        // +143 (2026-11-19): three new theme statics (monokai/catppuccin/kombat)
        // + the logo-contrast gate test — palette data rows + one gate, not
        // behaviour; splitting a palette table buys nothing.
        ("src/theme.rs", 1_088),
        // +35 (2026-11-19): cell_heat (inward→outward) + glow_bend (hot-core
        // white bend). New tests below pin both.
        ("src/fire.rs", 1_083),
    ];

    /// Files smaller than this are not budgeted: below it, a new file cannot be
    /// the monolith problem.
    const UNBUDGETED_MAX: usize = 1_000;

    fn source_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
    }

    fn count_lines(path: &Path) -> std::io::Result<usize> {
        Ok(fs::read_to_string(path)?.lines().count())
    }

    /// Every `.rs` file under `src`, relative to the crate root, with its line count.
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, usize>) {
        let Ok(read) = fs::read_dir(dir) else {
            return;
        };
        for entry in read.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, root, out);
                continue;
            }
            if path.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let rel = format!("src/{rel}");
            if let Ok(lines) = count_lines(&path) {
                out.insert(rel, lines);
            }
        }
    }

    #[test]
    fn no_source_file_outgrows_its_budget() {
        let root = source_root();
        let mut sizes = BTreeMap::new();
        walk(&root, &root, &mut sizes);
        assert!(
            sizes.len() > 20,
            "the walk found {} files — it must see the tree, or this gate is vacuous",
            sizes.len()
        );
        let budgets: BTreeMap<&str, usize> = BUDGETS.iter().copied().collect();
        let mut over = Vec::new();
        for (file, lines) in &sizes {
            if let Some(budget) = budgets.get(file.as_str()) {
                if lines > budget {
                    over.push(format!("{file}: {lines} > {budget}"));
                }
            } else if *lines > UNBUDGETED_MAX {
                over.push(format!(
                    "{file}: {lines} lines but not budgeted (add a BUDGETS entry)"
                ));
            }
        }
        assert!(
            over.is_empty(),
            "file-size ratchet tripped — move the new behaviour into a module:\n{}",
            over.join("\n")
        );
    }

    #[test]
    fn budgets_are_not_stale() {
        let root = source_root();
        let mut sizes = BTreeMap::new();
        walk(&root, &root, &mut sizes);
        // A file that shrank must have its budget lowered in the same breath;
        // otherwise the ratchet silently hands the room back.
        let stale: Vec<String> = BUDGETS
            .iter()
            .filter_map(|(file, budget)| {
                let lines = sizes.get(*file)?;
                (*lines < *budget).then(|| format!("{file}: {lines} < {budget} — lower the budget"))
            })
            .collect();
        assert!(stale.is_empty(), "stale budgets:\n{}", stale.join("\n"));
    }
}
