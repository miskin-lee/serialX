//! The output filter in the title bar, and the matcher behind it.
//!
//! Each tab has one: a pattern, two switches (regular expression, match case),
//! and the matcher compiled from them. Compilation happens when any of the
//! three changes rather than on every frame, and a literal pattern is escaped
//! and sent through the same `Regex`, so both modes share one Unicode-aware,
//! case-folding matcher. Both switches start off: what is typed is looked
//! for as typed, in either case. The find bar over the terminal (see
//! [`crate::find`]) is the same pattern and switches asked a different
//! question — *where* rather than *whether* — so it holds one of these too.
//!
//! What the filter does with a line that matches is a third switch, the
//! [`FilterMode`]: highlight, where every line stays and the ones that
//! match are washed in the accent, or mask, where the lines that match are
//! the only ones shown (see [`crate::mask`]).

use std::ops::Range;

use regex::{Regex, RegexBuilder};

/// What the filter does with the lines that match: tints them where they
/// stand, or shows them and them alone.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum FilterMode {
    /// Every line stays; the ones that match are washed in the accent.
    #[default]
    Highlight,
    /// Only the lines that match are drawn; the rest are held back.
    Mask,
}

#[derive(Clone, Debug)]
pub(crate) struct OutputFilter {
    pattern: String,
    use_regex: bool,
    match_case: bool,
    mode: FilterMode,
    /// `None` while the pattern is empty. Otherwise the matcher, or why the
    /// pattern would not compile.
    matcher: Option<Result<Regex, String>>,
    /// Goes up whenever the matcher is rebuilt, so what was matched with
    /// the last one can tell it is stale.
    generation: u64,
}

impl Default for OutputFilter {
    /// Literal and case folded: `AT+OK` is looked for as `AT+OK`, in
    /// either case, and the `.*` switch is there for `ERR|WARN`.
    fn default() -> Self {
        Self {
            pattern: String::new(),
            use_regex: false,
            match_case: false,
            mode: FilterMode::default(),
            matcher: None,
            generation: 0,
        }
    }
}

impl OutputFilter {
    pub(crate) fn pattern(&self) -> &str {
        &self.pattern
    }

    pub(crate) fn use_regex(&self) -> bool {
        self.use_regex
    }

    pub(crate) fn match_case(&self) -> bool {
        self.match_case
    }

    pub(crate) fn mode(&self) -> FilterMode {
        self.mode
    }

    /// Which matcher this is: another number, another set of answers.
    pub(crate) fn generation(&self) -> u64 {
        self.generation
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

    /// Switches between tinting the lines that match and showing only
    /// them. The matcher is untouched: the same lines match either way.
    pub(crate) fn toggle_mode(&mut self) {
        self.mode = match self.mode {
            FilterMode::Highlight => FilterMode::Mask,
            FilterMode::Mask => FilterMode::Highlight,
        };
    }

    /// Whether lines are being matched: a non-empty pattern that compiled.
    pub(crate) fn is_active(&self) -> bool {
        matches!(self.matcher, Some(Ok(_)))
    }

    /// Whether lines are being held back: the mask is on and there is a
    /// pattern to hold them back with. With no pattern, or a broken one,
    /// the mask shows everything, as the highlight tints nothing.
    pub(crate) fn masking(&self) -> bool {
        self.mode == FilterMode::Mask && self.is_active()
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
        self.generation += 1;
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
    use super::{FilterMode, OutputFilter};

    #[test]
    fn an_empty_pattern_holds_nothing_back() {
        let filter = OutputFilter::default();
        assert!(!filter.is_active());
        assert!(filter.matches("anything"));
        assert_eq!(filter.error(), None);
    }

    #[test]
    fn plain_text_is_looked_for_as_typed_in_either_case() {
        let mut filter = OutputFilter::default();
        assert!(!filter.use_regex());
        assert!(!filter.match_case());
        assert!(filter.set_pattern("err|warn"));
        assert!(filter.is_active());
        assert!(!filter.matches("[ERROR] sensor 3 timed out"), "no alternation yet");
        assert!(filter.matches("saw err|warn in the log"));
        assert!(
            !filter.set_pattern("err|warn"),
            "an unchanged pattern reports no change"
        );
        filter.toggle_regex();
        assert!(filter.matches("[ERROR] sensor 3 timed out"));
        assert!(filter.matches("Warn: low battery"));
        assert!(!filter.matches("OK"));
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
        assert!(!filter.matches("ATTTOK"));
        assert!(filter.matches("AT+OK"));
        filter.toggle_regex();
        assert!(filter.matches("ATTTOK"), "as a regex, + repeats the T");
    }

    #[test]
    fn a_broken_pattern_reports_itself_and_hides_nothing() {
        let mut filter = OutputFilter::default();
        filter.toggle_regex();
        filter.set_pattern("ERR(");
        assert!(!filter.is_active());
        assert_eq!(filter.error(), Some("Unclosed group"));
        assert!(filter.matches("anything"));
        filter.toggle_regex();
        assert_eq!(filter.error(), None, "the same text is a fine literal");
        assert!(filter.matches("ERR("));
        assert!(!filter.matches("ERROR"));
    }

    /// A matcher says where each occurrence is, literally or as a regex.
    #[test]
    fn a_matcher_locates_every_occurrence() {
        let mut find = OutputFilter::default();
        find.set_pattern("a.");
        assert_eq!(find.find_ranges("a. ab a."), vec![0..2, 6..8]);
        find.toggle_regex();
        assert_eq!(find.find_ranges("a. ab a."), vec![0..2, 3..5, 6..8]);
        find.set_pattern("b*");
        assert_eq!(find.find_ranges("abba"), vec![1..3], "empty matches are dropped");
        find.set_pattern("");
        assert!(find.find_ranges("anything").is_empty());
    }

    /// The mask holds lines back only with a pattern that compiles, and
    /// switching it leaves the matcher — and so its generation — alone.
    #[test]
    fn the_mask_needs_a_pattern_to_hold_anything_back() {
        let mut filter = OutputFilter::default();
        assert_eq!(filter.mode(), FilterMode::Highlight);
        filter.toggle_mode();
        assert_eq!(filter.mode(), FilterMode::Mask);
        assert!(!filter.masking(), "nothing to match with yet");
        let generation = filter.generation();
        filter.set_pattern("ERR");
        assert!(filter.masking());
        assert_ne!(filter.generation(), generation, "a new matcher");
        let generation = filter.generation();
        filter.toggle_mode();
        assert!(!filter.masking());
        assert_eq!(filter.generation(), generation, "the same matcher");
        filter.toggle_mode();
        filter.toggle_regex();
        filter.set_pattern("ERR(");
        assert!(!filter.masking(), "a broken pattern hides nothing");
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
