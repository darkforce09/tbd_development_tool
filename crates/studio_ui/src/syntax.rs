use egui::{Color32, FontFamily, FontId, Galley, Ui};
use std::str::FromStr;
use std::sync::{Arc, OnceLock};
use syntect::easy::HighlightLines;
use syntect::highlighting::{Color, FontStyle, ScopeSelectors, StyleModifier, Theme, ThemeItem, ThemeSet};
use syntect::parsing::{SyntaxReference, SyntaxSet};

use crate::colors::*;

/// Singleton holding the initialized syntect SyntaxSet and VS Code Dark+ Theme.
pub struct SyntaxHighlighter {
    pub syntax_set: SyntaxSet,
    pub theme: Theme,
}

static HIGHLIGHTER: OnceLock<SyntaxHighlighter> = OnceLock::new();

impl SyntaxHighlighter {
    /// Returns the global thread-safe SyntaxHighlighter instance.
    pub fn global() -> &'static Self {
        HIGHLIGHTER.get_or_init(Self::init)
    }

    fn init() -> Self {
        let syntax_set = SyntaxSet::load_defaults_newlines();
        let theme = Self::create_vscode_dark_theme();
        Self { syntax_set, theme }
    }

    /// Constructs the official VS Code Dark+ theme for syntect.
    fn create_vscode_dark_theme() -> Theme {
        let mut theme = Theme {
            name: Some("VS Code Dark+".to_string()),
            author: Some("Microsoft / Code Canvas".to_string()),
            settings: syntect::highlighting::ThemeSettings {
                foreground: Some(Color { r: 0xd4, g: 0xd4, b: 0xd4, a: 0xff }),
                background: Some(Color { r: 0x1e, g: 0x1e, b: 0x1e, a: 0xff }),
                caret: Some(Color { r: 0xae, g: 0xaf, b: 0xad, a: 0xff }),
                selection: Some(Color { r: 0x26, g: 0x4f, b: 0x78, a: 0xff }),
                line_highlight: Some(Color { r: 0x28, g: 0x28, b: 0x28, a: 0xff }),
                ..Default::default()
            },
            scopes: Vec::new(),
        };

        let rules = [
            // Comments
            ("comment, comment.line, comment.block, punctuation.definition.comment", Color { r: 0x6a, g: 0x99, b: 0x55, a: 0xff }, FontStyle::empty()),
            // Keywords
            ("keyword, storage.type, storage.modifier, keyword.operator.word", Color { r: 0x56, g: 0x9c, b: 0xd6, a: 0xff }, FontStyle::empty()),
            // Control Flow (if, else, match, return, for, while, async, await)
            ("keyword.control, keyword.control.flow, keyword.control.return", Color { r: 0xc5, g: 0x86, b: 0xc0, a: 0xff }, FontStyle::empty()),
            // Functions & Methods
            ("entity.name.function, support.function, meta.function-call", Color { r: 0xdc, g: 0xdc, b: 0xaa, a: 0xff }, FontStyle::empty()),
            // Types, Structs, Classes, Enums, Traits
            ("entity.name.type, entity.name.class, entity.name.struct, entity.name.enum, entity.name.trait, support.type, support.class", Color { r: 0x4e, g: 0xc9, b: 0xb0, a: 0xff }, FontStyle::empty()),
            // Strings
            ("string, string.quoted, string.template", Color { r: 0xce, g: 0x91, b: 0x78, a: 0xff }, FontStyle::empty()),
            // Numbers & Literals
            ("constant.numeric, constant.numeric.integer, constant.numeric.float", Color { r: 0xb5, g: 0xce, b: 0xa8, a: 0xff }, FontStyle::empty()),
            // Constants & Enums
            ("constant.language, constant.character, constant.other", Color { r: 0x4f, g: 0xc1, b: 0xff, a: 0xff }, FontStyle::empty()),
            // Variables & Identifiers
            ("variable, variable.other, variable.parameter, variable.language", Color { r: 0x9c, g: 0xdc, b: 0xfe, a: 0xff }, FontStyle::empty()),
            // Attributes, Preprocessor, Macros
            ("entity.other.attribute-name, meta.preprocessor, keyword.control.directive", Color { r: 0xc5, g: 0x86, b: 0xc0, a: 0xff }, FontStyle::empty()),
            // Markdown Headings
            ("markup.heading, entity.name.section", Color { r: 0x56, g: 0x9c, b: 0xd6, a: 0xff }, FontStyle::BOLD),
            // Markdown Links
            ("markup.underline.link", Color { r: 0x37, g: 0x94, b: 0xff, a: 0xff }, FontStyle::UNDERLINE),
            // Markdown Bold & Italic
            ("markup.bold", Color { r: 0xd4, g: 0xd4, b: 0xd4, a: 0xff }, FontStyle::BOLD),
            ("markup.italic", Color { r: 0xd4, g: 0xd4, b: 0xd4, a: 0xff }, FontStyle::ITALIC),
            // Markdown Code
            ("markup.raw, markup.raw.inline, markup.raw.block", Color { r: 0xce, g: 0x91, b: 0x78, a: 0xff }, FontStyle::empty()),
            // Delimiters & Punctuation
            ("punctuation, punctuation.separator, punctuation.terminator", Color { r: 0xd4, g: 0xd4, b: 0xd4, a: 0xff }, FontStyle::empty()),
        ];

        for (selector_str, color, font_style) in rules {
            if let Ok(scope) = ScopeSelectors::from_str(selector_str) {
                theme.scopes.push(ThemeItem {
                    scope,
                    style: StyleModifier {
                        foreground: Some(color),
                        background: None,
                        font_style: if font_style.is_empty() { None } else { Some(font_style) },
                    },
                });
            }
        }

        // If for any reason theme items were empty, fallback to base16 dark
        if theme.scopes.is_empty() {
            let ts = ThemeSet::load_defaults();
            if let Some(base) = ts.themes.get("base16-ocean.dark") {
                return base.clone();
            }
        }

        theme
    }

    /// Finds the syntax reference for the specified language string or file path.
    pub fn find_syntax(&self, lang_or_ext: &str) -> &SyntaxReference {
        let clean = lang_or_ext.trim_start_matches('.').to_lowercase();

        // 1. Check known aliases & language mappings
        let mapped_ext = match clean.as_str() {
            "rs" | "rust" => "rs",
            "py" | "python" => "py",
            "ts" | "typescript" | "tsx" => "js",
            "js" | "javascript" | "mjs" | "cjs" | "jsx" => "js",
            "c" | "h" => "c",
            "cpp" | "hpp" | "cc" | "hh" | "cxx" => "cpp",
            "cs" | "csharp" => "cs",
            "go" | "golang" => "go",
            "java" => "java",
            "kt" | "kotlin" => "java",
            "md" | "markdown" => "md",
            "toml" => "yaml",
            "json" => "json",
            "yaml" | "yml" => "yaml",
            "sh" | "bash" | "zsh" => "sh",
            "html" | "htm" => "html",
            "css" | "scss" => "css",
            "sql" => "sql",
            "xml" => "xml",
            "rb" | "ruby" => "rb",
            "php" => "php",
            "lua" => "lua",
            "proto" | "protobuf" => "c",
            "ens" | "es" | "enforce" => "c", // Enforce script maps closely to C/C++ TextMate grammar
            other => other,
        };

        // 2. Try by extension
        if let Some(syntax) = self.syntax_set.find_syntax_by_extension(mapped_ext) {
            return syntax;
        }

        // 3. Try by token / name
        if let Some(syntax) = self.syntax_set.find_syntax_by_token(mapped_ext) {
            return syntax;
        }

        // 4. Fallback to Plain Text
        self.syntax_set.find_syntax_plain_text()
    }

    /// Highlights a single line into a list of (Color32, text_token) pairs.
    pub fn highlight_line_tokens<'a>(&self, lang: &str, line: &'a str) -> Vec<(Color32, &'a str)> {
        if line.is_empty() {
            return Vec::new();
        }

        let syntax = self.find_syntax(lang);
        let mut highlighter = HighlightLines::new(syntax, &self.theme);

        // Highlight line with syntect
        let line_with_newline = if line.ends_with('\n') { line.to_string() } else { format!("{}\n", line) };

        if let Ok(ranges) = highlighter.highlight_line(&line_with_newline, &self.syntax_set) {
            let mut result = Vec::with_capacity(ranges.len());
            let mut current_offset = 0;

            for (style, token_str) in ranges {
                // Strip possible added newline
                let clean_token = token_str.trim_end_matches('\r').trim_end_matches('\n');
                if clean_token.is_empty() {
                    continue;
                }

                let len = clean_token.len();
                let end = (current_offset + len).min(line.len());
                if current_offset < end && current_offset < line.len() {
                    let slice = &line[current_offset..end];
                    let color = Color32::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b);
                    result.push((color, slice));
                    current_offset = end;
                }
            }

            if current_offset < line.len() {
                result.push((TEXT_PRIMARY, &line[current_offset..]));
            }

            if !result.is_empty() {
                return result;
            }
        }

        // Fallback: return entire line with primary text color
        vec![(TEXT_PRIMARY, line)]
    }

    /// Highlights full multi-line text into an egui LayoutJob with line wrapping and monospace font.
    pub fn highlight_layout_job(
        &self,
        text: &str,
        lang: &str,
        font_size: f32,
        wrap_width: f32,
    ) -> egui::text::LayoutJob {
        let mut job = egui::text::LayoutJob::default();
        job.wrap.max_width = wrap_width;

        let syntax = self.find_syntax(lang);
        let mut highlighter = HighlightLines::new(syntax, &self.theme);
        let font_id = FontId::new(font_size, FontFamily::Monospace);

        for line in text.split_inclusive('\n') {
            if let Ok(ranges) = highlighter.highlight_line(line, &self.syntax_set) {
                for (style, token_str) in ranges {
                    let color = Color32::from_rgb(style.foreground.r, style.foreground.g, style.foreground.b);
                    job.append(
                        token_str,
                        0.0,
                        egui::TextFormat { font_id: font_id.clone(), color, ..Default::default() },
                    );
                }
            } else {
                job.append(
                    line,
                    0.0,
                    egui::TextFormat { font_id: font_id.clone(), color: TEXT_PRIMARY, ..Default::default() },
                );
            }
        }

        job
    }
}

/// Resolves language from a file path or extension or hint.
pub fn detect_language(path_or_file: Option<&str>, hint: Option<&str>) -> String {
    if let Some(h) = hint {
        if !h.trim().is_empty() {
            return h.trim().to_lowercase();
        }
    }

    if let Some(p) = path_or_file {
        if let Some(ext) = std::path::Path::new(p).extension().and_then(|s| s.to_str()) {
            return ext.to_lowercase();
        }
    }

    "rs".to_string()
}

/// Creates a layouter function closure for egui::TextEdit to enable real-time VS Code syntax highlighting.
pub fn code_editor_layouter(lang: String) -> impl FnMut(&Ui, &dyn egui::TextBuffer, f32) -> Arc<Galley> {
    move |ui: &Ui, text: &dyn egui::TextBuffer, wrap_width: f32| {
        let text = text.as_str();
        let highlighter = SyntaxHighlighter::global();
        let font_size = 11.5;
        let job = highlighter.highlight_layout_job(text, &lang, font_size, wrap_width);
        ui.fonts_mut(|f| f.layout_job(job))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_syntax_resolution_languages() {
        let hl = SyntaxHighlighter::global();
        assert_eq!(hl.find_syntax("rs").name, "Rust");
        assert_eq!(hl.find_syntax("py").name, "Python");
        assert_eq!(hl.find_syntax("js").name, "JavaScript");
        assert_eq!(hl.find_syntax("c").name, "C");
        assert_eq!(hl.find_syntax("cpp").name, "C++");
        assert_eq!(hl.find_syntax("md").name, "Markdown");
        assert_eq!(hl.find_syntax("json").name, "JSON");
        assert_eq!(hl.find_syntax("yaml").name, "YAML");
    }

    #[test]
    fn test_highlight_line_tokens_rust() {
        let hl = SyntaxHighlighter::global();
        let tokens = hl.highlight_line_tokens("rs", "pub fn calculate_sum(a: usize, b: usize) -> usize {");
        assert!(!tokens.is_empty());
        // The first token should be "pub" or similar with a keyword color (blue in VS Code)
        let first_token = &tokens[0];
        assert!(first_token.1.starts_with("pub"));
        assert_eq!(first_token.0, VSCODE_KEYWORD);
    }

    #[test]
    fn test_highlight_line_tokens_python() {
        let hl = SyntaxHighlighter::global();
        let tokens = hl.highlight_line_tokens("py", "def process_data(items): # Process items");
        assert!(!tokens.is_empty());
        assert!(tokens.iter().any(|(_, t)| t.contains("def")));
    }
}
