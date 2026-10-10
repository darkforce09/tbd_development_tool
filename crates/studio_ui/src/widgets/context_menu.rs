/// What a card's context menu offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuItem {
    OpenOnDesk,
    ToggleExpand,
    ViewDocs,
    Center,
    ToggleWires,
    CopyPath,
}
