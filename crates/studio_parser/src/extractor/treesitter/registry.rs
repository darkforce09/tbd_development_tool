use tree_sitter::Language;

/// Languages extracted with tree-sitter grammars.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CodeLang {
    Bash,
    C,
    Cpp,
    CSharp,
    Dart,
    Go,
    Java,
    JavaScript,
    Kotlin,
    Lua,
    Php,
    Python,
    Ruby,
    Scala,
    Swift,
    Tsx,
    TypeScript,
    Zig,
}

impl CodeLang {
    pub const ALL: [CodeLang; 18] = [
        CodeLang::Bash,
        CodeLang::C,
        CodeLang::Cpp,
        CodeLang::CSharp,
        CodeLang::Dart,
        CodeLang::Go,
        CodeLang::Java,
        CodeLang::JavaScript,
        CodeLang::Kotlin,
        CodeLang::Lua,
        CodeLang::Php,
        CodeLang::Python,
        CodeLang::Ruby,
        CodeLang::Scala,
        CodeLang::Swift,
        CodeLang::Tsx,
        CodeLang::TypeScript,
        CodeLang::Zig,
    ];

    /// Grammar for a lowercased file extension. `.c` is decided by the caller (C vs Enforce Script).
    pub fn from_extension(ext: &str) -> Option<Self> {
        Some(match ext {
            "sh" | "bash" | "zsh" => CodeLang::Bash,
            "c" => CodeLang::C,
            // C headers parse fine with the C++ grammar, which also covers C++ headers.
            "h" | "cpp" | "cc" | "cxx" | "c++" | "hpp" | "hh" | "hxx" | "h++" | "ino" => CodeLang::Cpp,
            "cs" => CodeLang::CSharp,
            "dart" => CodeLang::Dart,
            "go" => CodeLang::Go,
            "java" => CodeLang::Java,
            "js" | "mjs" | "cjs" | "jsx" => CodeLang::JavaScript,
            "kt" | "kts" => CodeLang::Kotlin,
            "lua" => CodeLang::Lua,
            "php" | "phtml" => CodeLang::Php,
            "py" | "pyi" | "pyw" => CodeLang::Python,
            "rb" | "rake" | "gemspec" => CodeLang::Ruby,
            "scala" | "sc" => CodeLang::Scala,
            "swift" => CodeLang::Swift,
            "tsx" => CodeLang::Tsx,
            "ts" | "mts" | "cts" => CodeLang::TypeScript,
            "zig" => CodeLang::Zig,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            CodeLang::Bash => "Bash",
            CodeLang::C => "C",
            CodeLang::Cpp => "C++",
            CodeLang::CSharp => "C#",
            CodeLang::Dart => "Dart",
            CodeLang::Go => "Go",
            CodeLang::Java => "Java",
            CodeLang::JavaScript => "JavaScript",
            CodeLang::Kotlin => "Kotlin",
            CodeLang::Lua => "Lua",
            CodeLang::Php => "PHP",
            CodeLang::Python => "Python",
            CodeLang::Ruby => "Ruby",
            CodeLang::Scala => "Scala",
            CodeLang::Swift => "Swift",
            CodeLang::Tsx => "TSX",
            CodeLang::TypeScript => "TypeScript",
            CodeLang::Zig => "Zig",
        }
    }

    pub(super) fn language(self) -> Language {
        match self {
            CodeLang::Bash => tree_sitter_bash::LANGUAGE.into(),
            CodeLang::C => tree_sitter_c::LANGUAGE.into(),
            CodeLang::Cpp => tree_sitter_cpp::LANGUAGE.into(),
            CodeLang::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            CodeLang::Dart => tree_sitter_dart::LANGUAGE.into(),
            CodeLang::Go => tree_sitter_go::LANGUAGE.into(),
            CodeLang::Java => tree_sitter_java::LANGUAGE.into(),
            CodeLang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            CodeLang::Kotlin => tree_sitter_kotlin_ng::LANGUAGE.into(),
            CodeLang::Lua => tree_sitter_lua::LANGUAGE.into(),
            CodeLang::Php => tree_sitter_php::LANGUAGE_PHP.into(),
            CodeLang::Python => tree_sitter_python::LANGUAGE.into(),
            CodeLang::Ruby => tree_sitter_ruby::LANGUAGE.into(),
            CodeLang::Scala => tree_sitter_scala::LANGUAGE.into(),
            CodeLang::Swift => tree_sitter_swift::LANGUAGE.into(),
            CodeLang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            CodeLang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            CodeLang::Zig => tree_sitter_zig::LANGUAGE.into(),
        }
    }

    /// Extraction query; capture names are documented in `treesitter/mod.rs`.
    pub(super) fn query_source(self) -> &'static str {
        match self {
            CodeLang::Bash => include_str!("queries/bash.scm"),
            CodeLang::C => include_str!("queries/c.scm"),
            CodeLang::Cpp => include_str!("queries/cpp.scm"),
            CodeLang::CSharp => include_str!("queries/csharp.scm"),
            CodeLang::Dart => include_str!("queries/dart.scm"),
            CodeLang::Go => include_str!("queries/go.scm"),
            CodeLang::Java => include_str!("queries/java.scm"),
            CodeLang::JavaScript => include_str!("queries/javascript.scm"),
            CodeLang::Kotlin => include_str!("queries/kotlin.scm"),
            CodeLang::Lua => include_str!("queries/lua.scm"),
            CodeLang::Php => include_str!("queries/php.scm"),
            CodeLang::Python => include_str!("queries/python.scm"),
            CodeLang::Ruby => include_str!("queries/ruby.scm"),
            CodeLang::Scala => include_str!("queries/scala.scm"),
            CodeLang::Swift => include_str!("queries/swift.scm"),
            CodeLang::Tsx | CodeLang::TypeScript => include_str!("queries/typescript.scm"),
            CodeLang::Zig => include_str!("queries/zig.scm"),
        }
    }

    /// Keyword shown before function names in member signatures.
    pub fn fn_keyword(self) -> &'static str {
        match self {
            CodeLang::Python | CodeLang::Ruby | CodeLang::Scala => "def",
            CodeLang::Go | CodeLang::Swift => "func",
            CodeLang::Kotlin => "fun",
            CodeLang::JavaScript
            | CodeLang::TypeScript
            | CodeLang::Tsx
            | CodeLang::Php
            | CodeLang::Lua
            | CodeLang::Bash => "function",
            CodeLang::Zig => "fn",
            CodeLang::C | CodeLang::Cpp | CodeLang::CSharp | CodeLang::Java | CodeLang::Dart => "",
        }
    }

    /// What this language calls an interface-like type.
    pub fn interface_label(self) -> &'static str {
        match self {
            CodeLang::Swift => "protocol",
            CodeLang::Scala => "trait",
            _ => "interface",
        }
    }

    pub(super) fn private_by_underscore(self) -> bool {
        matches!(self, CodeLang::Python | CodeLang::Dart)
    }
}
