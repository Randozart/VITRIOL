//! SPQL (Spagyric Query Language) — the Officina command grammar.
//!
//! Keyword-first, pipe-delimited, non-Turing-complete:
//!
//! ```text
//! COMMAND  := [ "ASCENSUS" ">" ] [ "COMMIT" [ "overwrite" | "as" NAME ] ">" ]
//!             KEYWORD ">" TARGET [ARGS]
//! KEYWORD  := DESCRIBE | CENSUS | RECTIFY | DISSOLVE | COAGULATE | TEST | MAP
//!           | COMPILE | RECORD | STOP | PLAY | DISCARD | LOG | REVERT
//!           | UNDO | CLEAR | HELP
//! ```
//!
//! Uppercase keywords come first; lowercase targets/args follow; `>` is the
//! work conduit. `COMMIT >` applies a mutating probe; `COMMIT overwrite >`
//! modifies the active target in place; `COMMIT as "name" >` writes a clone.
//! `ASCENSUS >` marks the command as cloud-assisted (batch calibration).
//! Read-only keywords ignore the commit prefix. Parse errors carry a message
//! the REPL prints as-is.

/// A command keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keyword {
    /// Census of a layer/model (read-only).
    Describe,
    /// W0 value census of a layer (dead-lane %, entropy; read-only).
    Census,
    /// Record a firing transaction into a rectification mask.
    Rectify,
    /// Prune weights (weight surgery; P3 backend).
    Dissolve,
    /// Fold a normalizer into adjacent weights (P3 backend).
    Coagulate,
    /// Run a prompt through the active model (read-only).
    Test,
    /// Print the real system memory map (read-only).
    Map,
    /// Package grimoire + fingerprint + profile into a `.spagyr` bundle.
    Compile,
    /// Begin recording committed ops into a grimoire.
    Record,
    /// Stop recording and write the grimoire file.
    Stop,
    /// Parse + run a grimoire (probe by default, `COMMIT > PLAY` applies).
    Play,
    /// Delete a named rectification mask.
    Discard,
    /// List a mask's transaction history (read-only).
    Log,
    /// Remove one transaction from a mask (probe shows impact).
    Revert,
    /// Render the workshop how-to manual (read-only).
    Guide,
    /// Revert the last committed transformation.
    Undo,
    /// Clear the output scrollback.
    Clear,
    /// List available keywords.
    Help,
}

impl Keyword {
    /// The keyword's canonical uppercase text.
    pub fn as_str(self) -> &'static str {
        match self {
            Keyword::Describe => "DESCRIBE",
            Keyword::Census => "CENSUS",
            Keyword::Rectify => "RECTIFY",
            Keyword::Dissolve => "DISSOLVE",
            Keyword::Coagulate => "COAGULATE",
            Keyword::Test => "TEST",
            Keyword::Map => "MAP",
            Keyword::Compile => "COMPILE",
            Keyword::Record => "RECORD",
            Keyword::Stop => "STOP",
            Keyword::Play => "PLAY",
            Keyword::Discard => "DISCARD",
            Keyword::Log => "LOG",
            Keyword::Revert => "REVERT",
            Keyword::Guide => "GUIDE",
            Keyword::Undo => "UNDO",
            Keyword::Clear => "CLEAR",
            Keyword::Help => "HELP",
        }
    }

    /// True for keywords that are read-only (no `COMMIT >` needed).
    pub fn is_read_only(self) -> bool {
        matches!(
            self,
            Keyword::Describe
                | Keyword::Census
                | Keyword::Test
                | Keyword::Map
                | Keyword::Log
                | Keyword::Guide
                | Keyword::Help
                | Keyword::Clear
        )
    }

    /// True for keywords that mutate the base model or a mask — these need an
    /// explicit `COMMIT overwrite >` / `COMMIT as "name" >` (safety contract).
    pub fn needs_explicit_commit(self) -> bool {
        matches!(
            self,
            Keyword::Rectify
                | Keyword::Dissolve
                | Keyword::Coagulate
                | Keyword::Discard
                | Keyword::Revert
        )
    }
}

/// The explicit write target of a `COMMIT` prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommitKind {
    /// `COMMIT overwrite >` — modify the active target in place.
    Overwrite,
    /// `COMMIT as "name" >` — write a clone under a new name.
    SaveAs(String),
}

/// A parsed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// `COMMIT` was prefixed: apply instead of probe.
    pub commit: bool,
    /// The explicit write target when `commit` was not bare.
    pub commit_kind: Option<CommitKind>,
    /// `ASCENSUS >` was prefixed: cloud-assisted (batch calibration).
    pub cloud: bool,
    /// The keyword.
    pub keyword: Keyword,
    /// The target token (may be empty for STOP/UNDO/CLEAR/HELP).
    pub target: String,
    /// Remaining argument tokens.
    pub args: Vec<String>,
    /// The raw input line.
    pub raw: String,
}

/// Parse `line` into a command. Empty lines and `//` comments return `Err`
/// with `Empty` semantics — the REPL ignores them.
pub fn parse(line: &str) -> Result<Command, ParseError> {
    let raw = line.trim();
    if raw.is_empty() || raw.starts_with("//") {
        return Err(ParseError::Empty);
    }
    let mut rest = raw;
    let mut cloud = false;
    if let Some(after) = strip_prefix_keyword(rest, "ASCENSUS") {
        cloud = true;
        rest = after.trim();
    }
    let mut commit = false;
    let mut commit_kind = None;
    if let Some(after) = rest.strip_prefix("COMMIT") {
        commit = true;
        let mut after = after.trim_start();
        if let Some((kind, rest2)) = parse_commit_kind(after) {
            commit_kind = Some(kind);
            after = rest2.trim_start();
        }
        rest = after.strip_prefix('>').unwrap_or(after).trim();
    }
    let (keyword_text, after_kw) = match split_keyword(rest) {
        Some((kw, after)) => (kw, after),
        None => (rest, ""),
    };
    let keyword = parse_keyword(keyword_text)?;
    let (target, args) = split_target_args(after_kw);
    Ok(Command {
        commit,
        commit_kind,
        cloud,
        keyword,
        target,
        args,
        raw: raw.to_string(),
    })
}

/// Parse `overwrite` or `as "name"` off the commit head.
fn parse_commit_kind(s: &str) -> Option<(CommitKind, &str)> {
    let s = s.trim_start();
    if let Some(rest) = s.strip_prefix("overwrite") {
        return Some((CommitKind::Overwrite, rest));
    }
    if let Some(rest) = s.strip_prefix("as") {
        let rest = rest.trim_start();
        let quoted = rest.strip_prefix('"')?;
        let end = quoted.find('"')?;
        let name = quoted[..end].to_string();
        return Some((CommitKind::SaveAs(name), &quoted[end + 1..]));
    }
    None
}

/// A parse outcome other than a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// Empty/comment line — ignore.
    Empty,
    /// Unknown keyword.
    UnknownKeyword(String),
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Empty => Ok(()),
            ParseError::UnknownKeyword(k) => write!(f, "unknown keyword: {k}"),
        }
    }
}

/// If `s` starts with `KEYWORD` followed by `>`, return the remainder.
fn strip_prefix_keyword<'a>(s: &'a str, keyword: &str) -> Option<&'a str> {
    let s = s.trim_start();
    let rest = s.strip_prefix(keyword)?;
    let rest = rest.trim_start();
    rest.strip_prefix('>')
}

/// Split `KEYWORD >` off the head; returns (keyword_text, remainder).
fn split_keyword(s: &str) -> Option<(&str, &str)> {
    let idx = s.find('>')?;
    let head = s[..idx].trim();
    if head.is_empty() {
        return None;
    }
    Some((head, s[idx + 1..].trim()))
}

/// Map a keyword token to its Keyword.
fn parse_keyword(text: &str) -> Result<Keyword, ParseError> {
    let up = text.to_ascii_uppercase();
    let kw = match up.as_str() {
        "DESCRIBE" => Keyword::Describe,
        "CENSUS" => Keyword::Census,
        "RECTIFY" => Keyword::Rectify,
        "DISSOLVE" => Keyword::Dissolve,
        "COAGULATE" => Keyword::Coagulate,
        "TEST" => Keyword::Test,
        "MAP" => Keyword::Map,
        "COMPILE" => Keyword::Compile,
        "RECORD" => Keyword::Record,
        "STOP" => Keyword::Stop,
        "PLAY" => Keyword::Play,
        "DISCARD" => Keyword::Discard,
        "LOG" => Keyword::Log,
        "REVERT" => Keyword::Revert,
        "GUIDE" => Keyword::Guide,
        "UNDO" => Keyword::Undo,
        "CLEAR" => Keyword::Clear,
        "HELP" => Keyword::Help,
        _ => return Err(ParseError::UnknownKeyword(text.to_string())),
    };
    Ok(kw)
}

/// Split the post-pipe remainder into (target, args), honoring one leading
/// double-quoted target.
fn split_target_args(rest: &str) -> (String, Vec<String>) {
    let rest = rest.trim();
    if rest.is_empty() {
        return (String::new(), Vec::new());
    }
    if let Some(rest_after) = rest.strip_prefix('"') {
        if let Some(end) = rest_after.find('"') {
            let target = rest_after[..end].to_string();
            let tail = rest_after[end + 1..].trim();
            return (target, tokenize(tail));
        }
    }
    let mut it = rest.splitn(2, char::is_whitespace);
    let target = it.next().unwrap_or("").to_string();
    let tail = it.next().unwrap_or("").trim();
    (target, tokenize(tail))
}

/// Split whitespace-separated tokens.
fn tokenize(s: &str) -> Vec<String> {
    s.split_whitespace().map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_probe_and_commit_prune() {
        let c = parse("DISSOLVE > layer.12.mlp wanda 0.35").unwrap();
        assert!(!c.commit);
        assert_eq!(c.keyword, Keyword::Dissolve);
        assert_eq!(c.target, "layer.12.mlp");
        assert_eq!(c.args, vec!["wanda", "0.35"]);

        let c = parse("COMMIT > DISSOLVE > layer.12.mlp wanda 0.35").unwrap();
        assert!(c.commit);
        assert_eq!(c.keyword, Keyword::Dissolve);
    }

    #[test]
    fn parses_quoted_targets() {
        let c = parse("TEST > \"write a fast inverse square root in c\"").unwrap();
        assert_eq!(c.keyword, Keyword::Test);
        assert_eq!(c.target, "write a fast inverse square root in c");
        assert!(c.args.is_empty());

        let c = parse("COMPILE > \"vitriol-coder-32b\"").unwrap();
        assert_eq!(c.target, "vitriol-coder-32b");
    }

    #[test]
    fn keywords_are_case_insensitive() {
        let c = parse("describe > model").unwrap();
        assert_eq!(c.keyword, Keyword::Describe);
    }

    #[test]
    fn unknown_keyword_errors() {
        let e = parse("FROBNICATE > x").unwrap_err();
        assert!(matches!(e, ParseError::UnknownKeyword(_)));
    }

    #[test]
    fn empty_and_comments_ignored() {
        assert_eq!(parse(""), Err(ParseError::Empty));
        assert_eq!(parse("   "), Err(ParseError::Empty));
        assert_eq!(parse("// comment"), Err(ParseError::Empty));
    }

    #[test]
    fn missing_pipe_errors() {
        // No `>`: the whole line is treated as the keyword, which is not one.
        assert!(matches!(
            parse("DISSOLVE layer.12"),
            Err(ParseError::UnknownKeyword(_))
        ));
    }

    #[test]
    fn read_only_keywords_ignore_commit() {
        assert!(Keyword::Test.is_read_only());
        assert!(!Keyword::Dissolve.is_read_only());
        assert!(!Keyword::Compile.is_read_only());
    }

    #[test]
    fn stop_without_target_is_valid() {
        let c = parse("STOP").unwrap();
        assert_eq!(c.keyword, Keyword::Stop);
        assert!(c.target.is_empty());
    }

    #[test]
    fn commit_kinds_parse() {
        let c = parse("COMMIT overwrite > DISSOLVE > layer.12.mlp wanda 0.35").unwrap();
        assert!(c.commit);
        assert_eq!(c.commit_kind, Some(CommitKind::Overwrite));

        let c = parse("COMMIT as \"vitriol-vulkan\" > COMPILE > \"x\"").unwrap();
        assert!(c.commit);
        assert_eq!(
            c.commit_kind,
            Some(CommitKind::SaveAs("vitriol-vulkan".into()))
        );

        let c = parse("COMMIT > COMPILE > \"x\"").unwrap();
        assert!(c.commit);
        assert_eq!(c.commit_kind, None);
    }

    #[test]
    fn ascensus_modifier_sets_cloud() {
        let c = parse("ASCENSUS > RECTIFY > \"specialize in Vulkan\" 50").unwrap();
        assert!(c.cloud);
        assert_eq!(c.keyword, Keyword::Rectify);
        assert_eq!(c.target, "specialize in Vulkan");
        assert_eq!(c.args, vec!["50"]);

        let c = parse("RECTIFY > \"write a circular buffer\"").unwrap();
        assert!(!c.cloud);
        assert_eq!(c.keyword, Keyword::Rectify);
    }

    #[test]
    fn new_keywords_parse() {
        assert_eq!(
            parse("LOG > model vulkan_shaders").unwrap().keyword,
            Keyword::Log
        );
        assert_eq!(
            parse("REVERT > model vulkan_shaders 2").unwrap().keyword,
            Keyword::Revert
        );
        assert_eq!(
            parse("DISCARD > model vulkan_shaders").unwrap().keyword,
            Keyword::Discard
        );
    }

    #[test]
    fn rect_mask_target_and_args() {
        let c = parse("DESCRIBE > model vulkan_shaders").unwrap();
        assert_eq!(c.keyword, Keyword::Describe);
        assert_eq!(c.target, "model");
        assert_eq!(c.args, vec!["vulkan_shaders"]);
    }

    #[test]
    fn needs_explicit_commit_classification() {
        for kw in [
            Keyword::Rectify,
            Keyword::Dissolve,
            Keyword::Coagulate,
            Keyword::Discard,
            Keyword::Revert,
        ] {
            assert!(kw.needs_explicit_commit());
        }
        assert!(Keyword::Log.is_read_only());
        assert!(!Keyword::Compile.needs_explicit_commit());
        assert!(!Keyword::Test.needs_explicit_commit());
    }
}
