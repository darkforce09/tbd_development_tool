use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemVisibility {
    Public,
    Crate,
    Private,
}

impl ItemVisibility {
    pub fn is_public(&self) -> bool {
        matches!(self, ItemVisibility::Public)
    }

    pub fn badge_prefix(&self) -> &'static str {
        match self {
            ItemVisibility::Public => "PUB ",
            ItemVisibility::Crate => "CRATE ",
            ItemVisibility::Private => "",
        }
    }
}

#[derive(Debug, Clone)]
pub struct ParamInfo {
    pub name: String,
    pub type_str: String,
}

#[derive(Debug, Clone)]
pub struct FunctionItem {
    pub name: String,
    pub visibility: ItemVisibility,
    pub is_async: bool,
    pub is_method: bool,
    pub self_param: Option<String>,
    pub inputs: Vec<ParamInfo>,
    pub output: Option<String>,
    pub calls: Vec<String>,
    pub docs: String,
    pub line: usize,
    pub source_code: String,
}

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub type_str: String,
    pub visibility: ItemVisibility,
}

#[derive(Debug, Clone)]
pub struct StructItem {
    pub name: String,
    pub visibility: ItemVisibility,
    pub fields: Vec<FieldInfo>,
    pub derives: Vec<String>,
    pub docs: String,
    pub line: usize,
    pub source_code: String,
}

#[derive(Debug, Clone)]
pub struct EnumItem {
    pub name: String,
    pub visibility: ItemVisibility,
    pub variants: Vec<String>,
    pub docs: String,
    pub line: usize,
    pub source_code: String,
}

#[derive(Debug, Clone)]
pub struct TraitItem {
    pub name: String,
    pub visibility: ItemVisibility,
    pub methods: Vec<String>,
    pub docs: String,
    pub line: usize,
    pub source_code: String,
}

#[derive(Debug, Clone)]
pub struct ImplItem {
    pub target_type: String,
    pub trait_name: Option<String>,
    pub methods: Vec<FunctionItem>,
    pub line: usize,
}

#[derive(Debug, Clone)]
pub struct UseItem {
    pub path: String,
    pub items: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct ExtractedFile {
    pub file_path: PathBuf,
    pub relative_path: PathBuf,
    pub module_name: String,
    pub functions: Vec<FunctionItem>,
    pub structs: Vec<StructItem>,
    pub enums: Vec<EnumItem>,
    pub traits: Vec<TraitItem>,
    pub impls: Vec<ImplItem>,
    pub uses: Vec<UseItem>,
    pub parse_error: Option<String>,
    pub language: super::lang::SourceLang,
}

impl ExtractedFile {
    /// A file with no extracted items, e.g. unreadable or skipped.
    pub fn empty(file_path: &std::path::Path, rel_path: &std::path::Path, language: super::lang::SourceLang, parse_error: Option<String>) -> Self {
        Self {
            file_path: file_path.to_path_buf(),
            relative_path: rel_path.to_path_buf(),
            module_name: file_path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "mod".to_string()),
            functions: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
            traits: Vec::new(),
            impls: Vec::new(),
            uses: Vec::new(),
            parse_error,
            language,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExtractedCrate {
    pub name: String,
    pub root_path: PathBuf,
    pub files: Vec<ExtractedFile>,
}

#[derive(Debug, Clone)]
pub struct ExtractedProject {
    pub name: String,
    pub root_path: PathBuf,
    pub crates: Vec<ExtractedCrate>,
}
