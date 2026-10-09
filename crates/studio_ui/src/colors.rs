use egui::Color32;

// Canvas background
pub const CANVAS_BG: Color32 = Color32::from_rgb(0x12, 0x12, 0x14);
pub const CANVAS_GRID_DOT_MINOR: Color32 = Color32::from_rgb(0x1f, 0x1f, 0x27);
pub const CANVAS_GRID_DOT_MAJOR: Color32 = Color32::from_rgb(0x2d, 0x2d, 0x3a);

// Card palette
pub const CARD_BG: Color32 = Color32::from_rgb(0x1a, 0x1a, 0x20);
pub const CARD_BG_HOVER: Color32 = Color32::from_rgb(0x20, 0x20, 0x27);
pub const CARD_HEADER_BG: Color32 = Color32::from_rgb(0x23, 0x23, 0x2c);
pub const CARD_BORDER_NORMAL: Color32 = Color32::from_rgb(0x2e, 0x2e, 0x38);
pub const CARD_BORDER_HOVER: Color32 = Color32::from_rgb(0x40, 0x40, 0x50);
pub const CARD_BORDER_SELECTED: Color32 = Color32::from_rgb(0x63, 0x66, 0xf1);
pub const CARD_SHADOW: Color32 = Color32::from_black_alpha(80);

// Typography
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xf4, 0xf4, 0xf5);
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0xa1, 0xa1, 0xaa);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x71, 0x71, 0x7a);
pub const TEXT_HIGHLIGHT: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);

// Semantic Data Types
pub const TYPE_AUDIO: Color32 = Color32::from_rgb(0x06, 0xb6, 0xd4); // Cyan
pub const TYPE_AUDIO_GLOW: Color32 = Color32::from_rgb(0x22, 0xd3, 0xee);

pub const TYPE_VISION: Color32 = Color32::from_rgb(0xa8, 0x55, 0xf7); // Purple
pub const TYPE_VISION_GLOW: Color32 = Color32::from_rgb(0xc0, 0x84, 0xfc);

pub const TYPE_TEXT: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6); // Blue
pub const TYPE_TEXT_GLOW: Color32 = Color32::from_rgb(0x60, 0xa5, 0xfa);

pub const TYPE_STATE: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x0b); // Amber
pub const TYPE_STATE_GLOW: Color32 = Color32::from_rgb(0xfb, 0xbf, 0x24);

pub const TYPE_FLOW: Color32 = Color32::from_rgb(0x10, 0xb9, 0x81); // Emerald
pub const TYPE_FLOW_GLOW: Color32 = Color32::from_rgb(0x34, 0xd3, 0x99);

pub const TYPE_COMPOSITE: Color32 = Color32::from_rgb(0xd9, 0x46, 0xef); // Fuchsia
pub const TYPE_COMPOSITE_GLOW: Color32 = Color32::from_rgb(0xf0, 0xab, 0xfc);

pub const TYPE_DEFAULT: Color32 = Color32::from_rgb(0x9c, 0xa3, 0xaf); // Gray
pub const TYPE_DEFAULT_GLOW: Color32 = Color32::from_rgb(0xd1, 0xd5, 0xdb);

// Archetype Accents
pub const ARCHETYPE_INGRESS: Color32 = Color32::from_rgb(0x06, 0xb6, 0xd4);
pub const ARCHETYPE_COMPUTE: Color32 = Color32::from_rgb(0xa8, 0x55, 0xf7);
pub const ARCHETYPE_STATE: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x0b);
pub const ARCHETYPE_EGRESS: Color32 = Color32::from_rgb(0x10, 0xb9, 0x81);
pub const ARCHETYPE_FILE: Color32 = Color32::from_rgb(0x38, 0xbd, 0xf8);

// File Extension Badges (CodeSee Style)
pub const FILE_RS: Color32 = Color32::from_rgb(0xe0, 0x6c, 0x3a);      // Rust Orange
pub const FILE_ENFORCE: Color32 = Color32::from_rgb(0xf9, 0x73, 0x16); // Enforce Script Coral/Orange
pub const FILE_JS: Color32 = Color32::from_rgb(0xfa, 0xcc, 0x15);      // JavaScript Gold
pub const FILE_TS: Color32 = Color32::from_rgb(0x38, 0xbd, 0xf8);      // TypeScript Blue
pub const FILE_TOML: Color32 = Color32::from_rgb(0x94, 0xa3, 0xb8);    // Config Slate
pub const FILE_JSON: Color32 = Color32::from_rgb(0xf5, 0x9e, 0x0b);    // JSON Amber
pub const FILE_MD: Color32 = Color32::from_rgb(0x2d, 0xd4, 0xbf);      // Markdown Teal
pub const FILE_PY: Color32 = Color32::from_rgb(0x4a, 0xde, 0x80);      // Python Green
pub const FILE_C: Color32 = Color32::from_rgb(0x60, 0xa5, 0xfa);       // C Light Blue
pub const FILE_CPP: Color32 = Color32::from_rgb(0x02, 0x84, 0xc7);     // C++ Deep Blue
pub const FILE_CS: Color32 = Color32::from_rgb(0xa8, 0x55, 0xf7);      // C# Purple
pub const FILE_GO: Color32 = Color32::from_rgb(0x06, 0xb6, 0xd4);      // Go Cyan
pub const FILE_JAVA: Color32 = Color32::from_rgb(0xea, 0x58, 0x0c);    // Java Amber
pub const FILE_SH: Color32 = Color32::from_rgb(0x4a, 0xde, 0x80);      // Shell Green
pub const FILE_YAML: Color32 = Color32::from_rgb(0xf4, 0x3f, 0x5e);    // YAML Rose
pub const FILE_HTML: Color32 = Color32::from_rgb(0xf9, 0x73, 0x16);    // HTML Orange
pub const FILE_CSS: Color32 = Color32::from_rgb(0xa8, 0x55, 0xf7);     // CSS Purple
pub const FILE_DEFAULT: Color32 = Color32::from_rgb(0xa1, 0xa1, 0xaa);

pub fn file_extension_color(ext: &str) -> Color32 {
    let clean = ext.trim_start_matches('.');
    match clean.to_lowercase().as_str() {
        "rs" => FILE_RS,
        "ens" | "es" | "enforce" => FILE_ENFORCE,
        "js" | "mjs" | "cjs" | "jsx" => FILE_JS,
        "ts" | "mts" | "cts" | "tsx" => FILE_TS,
        "toml" => FILE_TOML,
        "json" => FILE_JSON,
        "md" | "markdown" => FILE_MD,
        "py" => FILE_PY,
        "c" | "h" => FILE_C,
        "cpp" | "hpp" | "cc" | "hh" | "cxx" => FILE_CPP,
        "cs" => FILE_CS,
        "go" => FILE_GO,
        "java" | "kt" => FILE_JAVA,
        "sh" | "bash" | "zsh" => FILE_SH,
        "yaml" | "yml" => FILE_YAML,
        "html" => FILE_HTML,
        "css" | "scss" => FILE_CSS,
        _ => FILE_DEFAULT,
    }
}

// Wires
pub const WIRE_DEFAULT: Color32 = Color32::from_rgb(0x4b, 0x55, 0x63);
pub const WIRE_ACTIVE: Color32 = Color32::from_rgb(0x81, 0x8c, 0xf8);
pub const WIRE_HOVER: Color32 = Color32::from_rgb(0xa5, 0xb4, 0xfc);
pub const WIRE_DISCONNECT: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);

// Socket
pub const SOCKET_RING_IDLE: Color32 = Color32::from_rgb(0x2d, 0x2d, 0x38);
pub const SOCKET_RING_HOVER: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);

// Overlay & toolbar
pub const PANEL_BG: Color32 = Color32::from_rgb(0x14, 0x14, 0x17);
pub const PANEL_BORDER: Color32 = Color32::from_rgb(0x27, 0x27, 0x2f);
pub const BADGE_BG: Color32 = Color32::from_rgb(0x27, 0x27, 0x33);

// CodeSee Group Clusters
pub const CLUSTER_TINTS: &[(Color32, Color32)] = &[
    (Color32::from_rgba_premultiplied(14, 30, 48, 60), Color32::from_rgba_premultiplied(56, 189, 248, 120)),   // Sky
    (Color32::from_rgba_premultiplied(35, 18, 48, 60), Color32::from_rgba_premultiplied(168, 85, 247, 120)),  // Purple
    (Color32::from_rgba_premultiplied(16, 38, 30, 60), Color32::from_rgba_premultiplied(52, 211, 153, 120)),   // Emerald
    (Color32::from_rgba_premultiplied(48, 30, 16, 60), Color32::from_rgba_premultiplied(251, 146, 60, 120)),  // Orange
    (Color32::from_rgba_premultiplied(40, 24, 40, 60), Color32::from_rgba_premultiplied(236, 72, 153, 120)),  // Pink
    (Color32::from_rgba_premultiplied(28, 28, 36, 60), Color32::from_rgba_premultiplied(148, 163, 184, 100)), // Slate
];
pub const CLUSTER_HEADER_BG: Color32 = Color32::from_rgb(0x1a, 0x1c, 0x24);

// Step Badges & Numbered Wires (CodeSee)
pub const STEP_BADGE_BG: Color32 = Color32::from_rgb(0x3b, 0x82, 0xf6);
pub const STEP_BADGE_TEXT: Color32 = Color32::from_rgb(0xff, 0xff, 0xff);

// Floating Action Toolbar (CodeSee)
pub const FLOATING_TOOLBAR_BG: Color32 = Color32::from_rgb(0x18, 0x19, 0x22);
pub const FLOATING_TOOLBAR_BORDER: Color32 = Color32::from_rgb(0x32, 0x35, 0x45);
pub const FLOATING_BTN_HOVER: Color32 = Color32::from_rgb(0x28, 0x2c, 0x3e);

// Code Canvas Editor Tokens
pub const CODE_EDITOR_BG: Color32 = Color32::from_rgb(0x0e, 0x11, 0x17);
pub const CODE_GUTTER_BG: Color32 = Color32::from_rgb(0x16, 0x1a, 0x23);
pub const CODE_GUTTER_TEXT: Color32 = Color32::from_rgb(0x52, 0x5a, 0x6e);
pub const CODE_LINE_HIGHLIGHT: Color32 = Color32::from_rgba_premultiplied(35, 45, 65, 80);
pub const CODE_KEYWORD: Color32 = Color32::from_rgb(0xf4, 0x72, 0xb6); // Pink
pub const CODE_FN: Color32 = Color32::from_rgb(0x60, 0xa5, 0xfa);      // Blue
pub const CODE_TYPE_COLOR: Color32 = Color32::from_rgb(0x34, 0xd3, 0x99); // Emerald
pub const CODE_STRING_COLOR: Color32 = Color32::from_rgb(0xfb, 0xbf, 0x24); // Amber
pub const CODE_COMMENT_COLOR: Color32 = Color32::from_rgb(0x6b, 0x72, 0x80); // Gray

// VS Code Dark+ Official Palette Tokens
pub const VSCODE_KEYWORD: Color32 = Color32::from_rgb(0x56, 0x9c, 0xd6);       // #569cd6 Blue (let, fn, pub, class, def)
pub const VSCODE_CONTROL_FLOW: Color32 = Color32::from_rgb(0xc5, 0x86, 0xc0);  // #c586c0 Purple (if, match, return, for)
pub const VSCODE_FUNCTION: Color32 = Color32::from_rgb(0xdc, 0xdc, 0xaa);      // #dcdcaa Yellow (function calls and defs)
pub const VSCODE_TYPE: Color32 = Color32::from_rgb(0x4e, 0xc9, 0xb0);          // #4ec9b0 Teal/Mint (Structs, Enums, Types)
pub const VSCODE_STRING: Color32 = Color32::from_rgb(0xce, 0x91, 0x78);        // #ce9178 Orange/Coral ("string")
pub const VSCODE_NUMBER: Color32 = Color32::from_rgb(0xb5, 0xce, 0xa8);        // #b5cea8 Light Sage Green (123, 0.5)
pub const VSCODE_COMMENT: Color32 = Color32::from_rgb(0x6a, 0x99, 0x55);       // #6a9955 Forest Green (//, /* */)
pub const VSCODE_VARIABLE: Color32 = Color32::from_rgb(0x9c, 0xdc, 0xfe);      // #9cdcfe Light Sky Blue (variables, fields)
pub const VSCODE_CONSTANT: Color32 = Color32::from_rgb(0x4f, 0xc1, 0xff);      // #4fc1ff Bright Blue (CONSTANTS, UPPERCASE)
pub const VSCODE_DELIMITER: Color32 = Color32::from_rgb(0xd4, 0xd4, 0xd4);     // #d4d4d4 Light Gray ({}, (), ;, ,)
pub const VSCODE_MACRO: Color32 = Color32::from_rgb(0xc5, 0x86, 0xc0);         // #c586c0 Preprocessor / Attributes
pub const VSCODE_LINK: Color32 = Color32::from_rgb(0x37, 0x94, 0xff);          // #3794ff VS Code Link Blue

/// Helper to tint or blend colors with opacity.
pub fn with_alpha(color: Color32, alpha: u8) -> Color32 {
    Color32::from_rgba_premultiplied(
        ((color.r() as u16 * alpha as u16) / 255) as u8,
        ((color.g() as u16 * alpha as u16) / 255) as u8,
        ((color.b() as u16 * alpha as u16) / 255) as u8,
        alpha,
    )
}
