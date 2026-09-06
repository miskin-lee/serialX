//! The output filter in the title bar, and the matcher behind it.
//!
//! Each tab has one: a pattern, two switches (regular expression, match case),
//! and the matcher compiled from them. Compilation happens when any of the
//! three changes rather than on every frame, and a literal pattern is escaped
//! and sent through the same `Regex`, so both modes share one Unicode-aware,
//! case-folding matcher. The find bar over the terminal (see [`crate::find`])
//! is the same pattern and switches asked a different question — *where*
//! rather than *whether* — so it holds one of these too, starting literal.

use std::ops::Range;

use regex::{Regex, RegexBuilder};

#[derive(Clone, Debug)]
pub(crate) struct OutputFilter {
    pattern: String,
    use_regex: bool,
    match_case: bool,
    /// `None` while the pattern is empty. Otherwise the matcher, or why the
    /// pattern would not compile.
    matcher: Option<Result<Regex, String>>,
}

impl Default for OutputFilter {
    /// Regular expressions on and case folded: the box is there to narrow a
    /// stream, and `ERR|WARN` is the first thing anyone types into it.
    fn default() -> Self {
        Self {
            pattern: String::new(),
            use_regex: true,
            match_case: false,
            matcher: None,
        }
    }
}

impl OutputFilter {
    /// A matcher with regular expressions off: what a find box starts as,
    /// where `.` and `+` are usually the characters looked for.
    pub(crate) fn literal() -> Self {
        Self {
            use_regex: false,
            ..Self::default()
        }
    }

    pub(crate) fn pattern(&self) -> &str {
        &self.pattern
    }

    pub(crate) fn use_regex(&self) -> bool {
        self.use_regex
    }

    pub(crate) fn match_case(&self) -> bool {
        self.match_case
    }

    /// Replaces the pattern; `true` when it differed from the current one.
    pub(crate) fn set_pattern(&mut self, pattern: &str) -> bool {
        if self.pattern == pattern {
            return false;
        }
        self.pattern = pattern.to_owned();
        self.recompile();
        true
    }

    pub(crate) fn toggle_regex(&mut self) {
        self.use_regex = !self.use_regex;
        self.recompile();
    }

    pub(crate) fn toggle_match_case(&mut self) {
        self.match_case = !self.match_case;
        self.recompile();
    }

    /// Whether lines are being held back: a non-empty pattern that compiled.
    pub(crate) fn is_active(&self) -> bool {
        matches!(self.matcher, Some(Ok(_)))
    }

    /// Why the pattern does not compile, in a phrase short enough for the box.
    pub(crate) fn error(&self) -> Option<&str> {
        match &self.matcher {
            Some(Err(message)) => Some(message),
            _ => None,
        }
    }

    /// Whether a line with this text stays visible. A pattern that does not
    /// compile hides nothing: a typo must never look like a device going quiet.
    pub(crate) fn matches(&self, text: &str) -> bool {
        match &self.matcher {
            Some(Ok(regex)) => regex.is_match(text),
            _ => true,
        }
    }

    /// Where the pattern occurs in a text, as byte ranges, left to right.
    /// Empty matches — what `a*` finds between every two characters — are
    /// left out, since there is nothing there to show. Nothing while the
    /// pattern is empty or broken.
    pub(crate) fn find_ranges(&self, text: &str) -> Vec<Range<usize>> {
        match &self.matcher {
            Some(Ok(regex)) => regex
                .find_iter(text)
                .map(|found| found.range())
                .filter(|range| !range.is_empty())
                .collect(),
            _ => Vec::new(),
        }
    }

    fn recompile(&mut self) {
        if self.pattern.is_empty() {
            self.matcher = None;
            return;
        }
        let source = if self.use_regex {
            self.pattern.clone()
        } else {
            regex::escape(&self.pattern)
        };
        self.matcher = Some(
            RegexBuilder::new(&source)
                .case_insensitive(!self.match_case)
                .build()
                .map_err(|error| summarize_regex_error(&error)),
        );
    }
}

/// The last line of a `regex` error, which is the one that names the problem;
/// the lines before it repeat the pattern with a caret under the bad spot.
fn summarize_regex_error(error: &regex::Error) -> String {
    let text = error.to_string();
    let last = text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("invalid pattern");
    let message = last.trim().trim_start_matches("error: ");
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => "Invalid pattern".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::OutputFilter;

    #[test]
    fn an_empty_pattern_holds_nothing_back() {
        let filter = OutputFilter::default();
        assert!(!filter.is_active());
        assert!(filter.matches("anything"));
        assert_eq!(filter.error(), None);
    }

    #[test]
    fn regular_expressions_are_on_by_default_and_case_folded() {
        let mut filter = OutputFilter::default();
        assert!(filter.set_pattern("err|warn"));
        assert!(filter.is_active());
        assert!(filter.matches("[ERROR] sensor 3 timed out"));
        assert!(filter.matches("Warn: low battery"));
        assert!(!filter.matches("OK"));
        assert!(
            !filter.set_pattern("err|warn"),
            "an unchanged pattern reports no change"
        );
    }

    #[test]
    fn match_case_stops_folding() {
        let mut filter = OutputFilter::default();
        filter.set_pattern("ok");
        filter.toggle_match_case();
        assert!(!filter.matches("OK"));
        assert!(filter.matches("ok"));
    }

    #[test]
    fn literal_mode_takes_metacharacters_at_face_value() {
        let mut filter = OutputFilter::default();
        filter.set_pattern("AT+OK");
        assert!(filter.matches("ATTTOK"), "as a regex, + repeats the T");
        filter.toggle_regex();
        assert!(!filter.matches("ATTTOK"));
        assert!(filter.matches("AT+OK"));
    }

    #[test]
    fn a_broken_pattern_reports_itself_and_hides_nothing() {
        let mut filter = OutputFilter::default();
        filter.set_pattern("ERR(");
        assert!(!filter.is_active());
        assert_eq!(filter.error(), Some("Unclosed group"));
        assert!(filter.matches("anything"));
        filter.toggle_regex();
        assert_eq!(filter.error(), None, "the same text is a fine literal");
        assert!(filter.matches("ERR("));
        assert!(!filter.matches("ERROR"));
    }

    /// A find starts literal, and says where each occurrence is.
    #[test]
    fn a_literal_matcher_locates_every_occurrence() {
        let mut find = OutputFilter::literal();
        assert!(!find.use_regex());
        find.set_pattern("a.");
        assert_eq!(find.find_ranges("a. ab a."), vec![0..2, 6..8]);
        find.toggle_regex();
        assert_eq!(find.find_ranges("a. ab a."), vec![0..2, 3..5, 6..8]);
        find.set_pattern("b*");
        assert_eq!(find.find_ranges("abba"), vec![1..3], "empty matches are dropped");
        find.set_pattern("");
        assert!(find.find_ranges("anything").is_empty());
    }

    #[test]
    fn clearing_the_pattern_deactivates_the_filter() {
        let mut filter = OutputFilter::default();
        filter.set_pattern("OK");
        assert!(filter.is_active());
        filter.set_pattern("");
        assert!(!filter.is_active());
        assert!(filter.matches("ERROR"));
    }
}
