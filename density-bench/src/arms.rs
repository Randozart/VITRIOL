// Request arms: what a run asks the *engine* to do differently.
//
// Two things were called "arm" before, and conflating them was a defect:
//
//   * a **request style** (grammar, and later telegraphic/codebook) changes the
//     request body;
//   * an **engine arm** (the `sweep` matrix) changes flags/env and needs its own
//     process.
//
// `sweep` used to hand its engine label to the request builder, so an arm named
// "grammar" inside a sweep matched nothing and measured the baseline while the
// report said otherwise. A style is now a type, not a string, and both axes can
// be combined: engine knob × request style.

/// How a request body is shaped. `Baseline` is byte-identical to a plain call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestStyle {
    /// Plain request: system + user, no `response_format`.
    Baseline,
    /// Attach the shape's JSON schema as a `response_format` (E1: helps only when
    /// the schema has structure; a flat schema pays ~13 % more tokens).
    Grammar,
}

impl RequestStyle {
    /// Parse the CLI name. Returns `None` for an unknown name so the caller can
    /// refuse rather than silently measuring the baseline.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "baseline" => Some(Self::Baseline),
            "grammar" => Some(Self::Grammar),
            _ => None,
        }
    }

    /// The name recorded in artifacts and printed in the table.
    pub fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Grammar => "grammar",
        }
    }

    /// Names accepted by `--arm`, for error messages and `--help`.
    pub fn names() -> &'static str {
        "baseline, grammar"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_accepted_name_round_trips() {
        for name in ["baseline", "grammar"] {
            let style = RequestStyle::from_name(name).expect("declared name parses");
            assert_eq!(style.name(), name);
        }
    }

    #[test]
    fn an_unknown_style_is_refused_rather_than_silently_measured_as_baseline() {
        // This is the regression that started the refactor: "telegraphic" must not
        // fall through to a plain request and report a meaningless win.
        assert_eq!(RequestStyle::from_name("telegraphic"), None);
        assert_eq!(RequestStyle::from_name(""), None);
    }
}
