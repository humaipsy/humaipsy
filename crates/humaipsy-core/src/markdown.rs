//! Minimal inline Markdown for chat content.
//!
//! Only `**bold**` is interpreted: assistant replies routinely contain it, and
//! leaving the markers visible in the transcript is a rendering bug rather than
//! a faithful view of the text. Everything else (single `*`, `_`, lists, code
//! fences, links) is deliberately left untouched — this is not a Markdown
//! engine, and pretending to be one would silently mangle code snippets and
//! mathematical notation.
//!
//! The parser lives in `humaipsy-core` rather than in the egui crate so it can
//! be unit tested without a window, and so the local web dashboard and a future
//! CLI can agree on the same rules.
//!
//! Rules:
//! - A bold run is the text between a matching pair of `**` markers.
//! - An unmatched `**` is preserved literally, so a half-typed message never
//!   loses characters.
//! - A bold run may span newlines.
//! - Adjacent runs with the same style are merged, and empty runs are dropped.

/// One run of same-styled inline text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlineSegment {
    pub text: String,
    pub bold: bool,
}

impl InlineSegment {
    fn push_to(out: &mut Vec<InlineSegment>, text: &str, bold: bool) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = out.last_mut()
            && last.bold == bold
        {
            last.text.push_str(text);
            return;
        }
        out.push(InlineSegment {
            text: text.to_string(),
            bold,
        });
    }
}

/// Split `input` into plain and bold runs, removing the `**` markers.
///
/// The returned segments concatenate back to the original text minus the
/// matched `**` markers (and minus nothing at all when no pair is matched).
pub fn inline_segments(input: &str) -> Vec<InlineSegment> {
    let mut out = Vec::new();
    let mut rest = input;

    while let Some(open) = rest.find("**") {
        InlineSegment::push_to(&mut out, &rest[..open], false);
        let after_open = &rest[open + 2..];

        let Some(close) = after_open.find("**") else {
            // Unmatched opener: keep it (and everything after it) verbatim.
            InlineSegment::push_to(&mut out, &rest[open..], false);
            return out;
        };

        if close == 0 {
            // `****` is not a bold run; keep the four markers as literal text.
            InlineSegment::push_to(&mut out, "****", false);
            rest = &rest[open + 4..];
            continue;
        }

        InlineSegment::push_to(&mut out, &after_open[..close], true);
        rest = &after_open[close + 2..];
    }

    InlineSegment::push_to(&mut out, rest, false);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> InlineSegment {
        InlineSegment {
            text: text.to_string(),
            bold: false,
        }
    }

    fn bold(text: &str) -> InlineSegment {
        InlineSegment {
            text: text.to_string(),
            bold: true,
        }
    }

    #[test]
    fn plain_text_is_one_plain_segment() {
        assert_eq!(inline_segments("just words"), vec![plain("just words")]);
    }

    #[test]
    fn empty_input_has_no_segments() {
        assert!(inline_segments("").is_empty());
    }

    #[test]
    fn a_single_bold_run_loses_its_markers() {
        assert_eq!(
            inline_segments("I care about **aesthetics** a lot"),
            vec![plain("I care about "), bold("aesthetics"), plain(" a lot")]
        );
    }

    #[test]
    fn multiple_bold_runs_are_separate() {
        assert_eq!(
            inline_segments("**one** and **two**"),
            vec![bold("one"), plain(" and "), bold("two")]
        );
    }

    #[test]
    fn adjacent_bold_runs_merge() {
        assert_eq!(
            inline_segments("**a****b**"),
            vec![bold("ab")]
        );
    }

    #[test]
    fn unmatched_marker_is_kept_literally() {
        assert_eq!(
            inline_segments("half **typed"),
            vec![plain("half **typed")]
        );
        assert_eq!(
            inline_segments("**a** then **b"),
            vec![bold("a"), plain(" then **b")]
        );
    }

    #[test]
    fn a_bold_run_may_span_newlines() {
        assert_eq!(
            inline_segments("**line one\nline two**"),
            vec![bold("line one\nline two")]
        );
    }

    #[test]
    fn four_markers_are_not_a_bold_run() {
        assert_eq!(inline_segments("****"), vec![plain("****")]);
    }

    #[test]
    fn underscores_and_single_asterisks_are_untouched() {
        assert_eq!(
            inline_segments("a *b* c _d_ e"),
            vec![plain("a *b* c _d_ e")]
        );
    }
}
