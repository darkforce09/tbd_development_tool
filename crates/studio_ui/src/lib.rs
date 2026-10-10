pub mod code_view;
pub mod colors;
pub mod markdown;
pub mod syntax;
pub mod theme;
pub mod widgets;

pub use code_view::{
    highlight_document, highlights_for, CodeDocument, CodeView, CodeViewResponse, GutterMark, GutterMarkKind,
    Highlights, STALE_TIP,
};
pub use colors as color_tokens;
pub use colors::*;
pub use markdown::*;
pub use syntax::*;
pub use theme::apply_theme;
pub use widgets::*;
