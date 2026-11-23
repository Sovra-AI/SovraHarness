//! Syntax colour for saved work.
//!
//! GPUI colours chat fences with syntect, but that function is private to
//! `gpui-component`. Saved work paints with this highlighter instead. It reads
//! the same syntax collection and turns scope names into the highlight theme
//! the app already built for light and dark, so a keyword and a string follow
//! that theme rather than one flat ink colour.

use std::cell::{Cell, RefCell};
use std::ops::Range;
use std::sync::LazyLock;

use gpui_kit::HighlightStyle;
use gpui_kit::component::highlighter::HighlightTheme;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

/// What the saved-work highlighter last coloured.
///
/// Headless windows do not report glyph colour. A test reads this after the
/// real preview lays out, and fails when the block named no language.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedWorkHighlight {
    /// Fence language the block carried. Empty when the piece named none.
    pub language: String,
    /// Source the highlighter saw, without the fence.
    pub code: String,
    /// Inclusive style runs. A run with no colour is ordinary ink.
    pub styles: Vec<(Range<usize>, HighlightStyle)>,
}

thread_local! {
    static LAST: RefCell<Option<SavedWorkHighlight>> = const { RefCell::new(None) };
    static CALLS: Cell<u32> = const { Cell::new(0) };
}

/// Takes the highlight the preview or a source card produced, if it named a language.
pub fn take_saved_work_highlight() -> Option<SavedWorkHighlight> {
    LAST.with(|slot| slot.borrow_mut().take())
}

/// How many times the saved-work highlighter has run on this thread.
pub fn saved_work_highlight_calls() -> u32 {
    CALLS.with(Cell::get)
}

/// Styles for one piece of source. Unknown languages and a blank name stay uncoloured.
pub(crate) fn highlight_code(
    language: &str,
    code: &str,
    theme: &HighlightTheme,
) -> Vec<(Range<usize>, HighlightStyle)> {
    CALLS.with(|calls| calls.set(calls.get().saturating_add(1)));
    // A blank or unknown language stays uncoloured, but the call is still
    // recorded so a test can tell a code block with no language from a block
    // that never reached the highlighter.
    let styles = find_syntax(language)
        .filter(|_| !language.is_empty())
        .map(|syntax| resolve_styles(&parse(syntax, code), &(0..code.len()), theme))
        .unwrap_or_default();
    LAST.with(|slot| {
        *slot.borrow_mut() = Some(SavedWorkHighlight {
            language: language.to_owned(),
            code: code.to_owned(),
            styles: styles.clone(),
        });
    });
    styles
}

static SYNTAXES: LazyLock<SyntaxSet> = LazyLock::new(two_face::syntax::extra_newlines);

fn find_syntax(language: &str) -> Option<&'static SyntaxReference> {
    let syntaxes = &*SYNTAXES;
    let canonical = canonical_language(language);
    let alias = match canonical.as_str() {
        "text" => return None,
        "csharp" => "cs",
        "make" => "Makefile",
        "markdown_inline" => "md",
        "embedded_template" | "ejs" => "erb",
        other => other,
    };
    [language, canonical.as_str(), alias]
        .into_iter()
        .find_map(|token| syntaxes.find_syntax_by_token(token))
        .filter(|syntax| syntax.name != "Plain Text")
}

fn canonical_language(name: &str) -> String {
    let name = name.to_lowercase();
    match name.as_str() {
        "plain" | "plaintext" => "text".to_owned(),
        "jsonc" => "json".to_owned(),
        "sh" => "bash".to_owned(),
        "c++" => "cpp".to_owned(),
        "cs" => "csharp".to_owned(),
        "scss" => "css".to_owned(),
        "ex" => "elixir".to_owned(),
        "js" => "javascript".to_owned(),
        "kt" | "kts" | "ktm" => "kotlin".to_owned(),
        "makefile" => "make".to_owned(),
        "md" | "mdx" => "markdown".to_owned(),
        "markdown-inline" => "markdown_inline".to_owned(),
        "php3" | "php4" | "php5" | "phtml" => "php".to_owned(),
        "protobuf" => "proto".to_owned(),
        "py" | "pyi" => "python".to_owned(),
        "rb" => "ruby".to_owned(),
        "rs" => "rust".to_owned(),
        "ts" => "typescript".to_owned(),
        "yml" => "yaml".to_owned(),
        _ => name,
    }
}

fn parse(syntax: &SyntaxReference, text: &str) -> Vec<(Range<usize>, &'static str)> {
    let syntaxes = &*SYNTAXES;
    let mut parser = ParseState::new(syntax);
    let mut scopes = ScopeStack::new();
    let mut names = std::collections::HashMap::<Scope, Option<&'static str>>::new();
    let mut highlights = Vec::new();
    let mut offset = 0;

    for line in LinesWithEndings::from(text) {
        if let Ok(operations) = parser.parse_line(line, syntaxes) {
            let mut cursor = 0;
            for (index, operation) in operations {
                push_span(
                    offset + cursor..offset + index,
                    &scopes,
                    &mut names,
                    &mut highlights,
                );
                let _ = scopes.apply(&operation);
                cursor = index;
            }
            push_span(
                offset + cursor..offset + line.len(),
                &scopes,
                &mut names,
                &mut highlights,
            );
        }
        offset += line.len();
    }
    highlights
}

fn push_span(
    range: Range<usize>,
    scopes: &ScopeStack,
    names: &mut std::collections::HashMap<Scope, Option<&'static str>>,
    highlights: &mut Vec<(Range<usize>, &'static str)>,
) {
    if range.is_empty() {
        return;
    }
    // The innermost named scope wins, so a string's quotes colour as the string.
    let name = scopes
        .scopes
        .iter()
        .rev()
        .find_map(|scope| *names.entry(*scope).or_insert_with(|| semantic_name(*scope)));
    let Some(name) = name else {
        return;
    };
    match highlights.last_mut() {
        Some((last, last_name)) if last.end == range.start && *last_name == name => {
            last.end = range.end;
        }
        _ => highlights.push((range, name)),
    }
}

fn semantic_name(scope: Scope) -> Option<&'static str> {
    let scope = scope.build_string();
    let is = |prefix: &str| {
        scope == prefix
            || scope
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with('.'))
    };
    if is("punctuation.definition.string")
        || is("punctuation.definition.comment")
        || is("invalid")
        || is("meta")
        || is("source")
        || is("text")
    {
        return None;
    }
    Some(
        if is("comment.block.documentation") || is("comment.line.documentation") {
            "comment.doc"
        } else if is("comment") {
            "comment"
        } else if is("constant.character.escape") {
            "string.escape"
        } else if is("string.regexp") {
            "string.regex"
        } else if is("string") {
            "string"
        } else if is("constant.numeric") {
            "number"
        } else if is("constant.language.boolean") {
            "boolean"
        } else if is("constant") || is("support.constant") || is("entity.name.constant") {
            "constant"
        } else if is("keyword.operator") {
            "operator"
        } else if is("keyword.control.import") || is("meta.preprocessor") {
            "preproc"
        } else if is("keyword") || is("storage.modifier") {
            "keyword"
        } else if is("storage.type") {
            storage_type_name(&scope)
        } else if is("entity.name.function")
            || is("support.function")
            || is("variable.function")
            || is("meta.function-call")
        {
            "function"
        } else if is("entity.name.type")
            || is("entity.name.class")
            || is("entity.name.struct")
            || is("entity.name.enum")
            || is("entity.name.trait")
            || is("entity.name.interface")
            || is("entity.other.inherited-class")
            || is("support.type")
            || is("support.class")
        {
            "type"
        } else if is("entity.name.tag") {
            "tag"
        } else if is("entity.other.attribute-name") {
            "attribute"
        } else if is("entity.name.label") {
            "label"
        } else if is("entity.name.section") || is("markup.heading") {
            "title"
        } else if is("variable.language") {
            "variable.special"
        } else if is("variable.other.member")
            || is("variable.other.property")
            || is("support.variable.property")
            || is("meta.mapping.key")
        {
            "property"
        } else if is("variable") {
            "variable"
        } else if is("markup.bold") {
            "emphasis.strong"
        } else if is("markup.italic") {
            "emphasis"
        } else if is("markup.underline.link") {
            "link_uri"
        } else if is("markup.raw") {
            "text.literal"
        } else if is("markup.list") {
            "punctuation.list_marker"
        } else if is("punctuation.section") || is("punctuation.definition.block") {
            "punctuation.bracket"
        } else if is("punctuation.separator") || is("punctuation.terminator") {
            "punctuation.delimiter"
        } else if is("punctuation") {
            "punctuation"
        } else {
            return None;
        },
    )
}

fn storage_type_name(scope: &str) -> &'static str {
    if [
        "class",
        "enum",
        "function",
        "impl",
        "interface",
        "module",
        "namespace",
        "struct",
        "trait",
        "type",
        "let",
        "var",
        "const",
    ]
    .iter()
    .any(|word| scope.contains(word))
    {
        "keyword"
    } else {
        "type"
    }
}

fn resolve_styles(
    highlights: &[(Range<usize>, &'static str)],
    range: &Range<usize>,
    theme: &HighlightTheme,
) -> Vec<(Range<usize>, HighlightStyle)> {
    let first = highlights.partition_point(|(highlight, _)| highlight.end <= range.start);
    let mut runs = Vec::new();
    let mut cursor = range.start;
    for (highlight, name) in &highlights[first..] {
        if highlight.start >= range.end {
            break;
        }
        let start = highlight.start.max(range.start);
        let end = highlight.end.min(range.end);
        if start >= end || end <= cursor {
            continue;
        }
        if cursor < start {
            runs.push((cursor..start, HighlightStyle::default()));
        }
        runs.push((start..end, theme.style(name).unwrap_or_default()));
        cursor = end;
    }
    if cursor < range.end {
        runs.push((cursor..range.end, HighlightStyle::default()));
    }
    runs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colour_of(
        styles: &[(Range<usize>, HighlightStyle)],
        code: &str,
        needle: &str,
    ) -> Option<gpui_kit::Hsla> {
        let start = code.find(needle).expect("needle");
        let end = start + needle.len();
        styles
            .iter()
            .find(|(range, style)| {
                range.start <= start && range.end >= end && style.color.is_some()
            })
            .and_then(|(_, style)| style.color)
    }

    #[test]
    fn rust_keyword_and_string_follow_the_theme() {
        let code = "fn main() { let bed = \"sunny\"; }";
        let dark = HighlightTheme::default_dark();
        let light = HighlightTheme::default_light();
        let dark_styles = highlight_code("rust", code, dark.as_ref());
        let light_styles = highlight_code("rust", code, light.as_ref());
        let dark_keyword = colour_of(&dark_styles, code, "fn").expect("dark keyword");
        let dark_string = colour_of(&dark_styles, code, "\"sunny\"").expect("dark string");
        let light_keyword = colour_of(&light_styles, code, "fn").expect("light keyword");
        assert_ne!(dark_keyword, dark_string);
        assert_eq!(
            dark_keyword,
            dark.style("keyword")
                .and_then(|style| style.color)
                .expect("keyword role")
        );
        assert_eq!(
            dark_string,
            dark.style("string")
                .and_then(|style| style.color)
                .expect("string role")
        );
        assert_ne!(dark_keyword, light_keyword);
        let unknown = highlight_code("notes", code, dark.as_ref());
        let unnamed = highlight_code("", code, dark.as_ref());
        assert_eq!(
            unknown.len(),
            0,
            "an unknown language stays plain: {unknown:?}"
        );
        assert_eq!(
            unnamed.len(),
            0,
            "a blank language stays plain: {unnamed:?}"
        );
    }
}
