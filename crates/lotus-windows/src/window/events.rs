#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WindowEvent {
    Resized { width: u32, height: u32 },
    DpiChanged { dpi: u32 },
    PlacementRefreshRequested,
    Pointer(PointerEvent),
    ContextMenuRequested(DockContextRequest),
    Search(SearchEvent),
    Settings(SettingsEvent),
    ContextMenu(ContextMenuEvent),
    Switcher(SwitcherEvent),
    AnimationFrame,
    StatusRefreshRequested,
    RenderRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsEvent {
    Resized { width: u32, height: u32 },
    DpiChanged { dpi: u32 },
    RenderRequested,
    PointerMoved { x: i32, y: i32 },
    PointerLeft,
    PointerPressed { x: i32, y: i32 },
    PointerReleased { x: i32, y: i32 },
    CloseRequested,
    KeyPressed(SettingsKey),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsKey {
    Escape,
    Enter,
    Tab { reverse: bool },
    Left,
    Right,
    Up,
    Down,
    Space,
    Save,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockContextRequest {
    Pointer {
        screen: SignedPoint,
        client: SignedPoint,
    },
    Keyboard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SignedPoint {
    pub x: i32,
    pub y: i32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PointerEvent {
    Moved { x: i32, y: i32 },
    Left,
    LeftButtonPressed { x: i32, y: i32 },
    LeftButtonReleased { x: i32, y: i32 },
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchEvent {
    TextInput(char),
    Edit(SearchEdit),
    PasteRequested,
    MoveSelection(SelectionDirection),
    PointerMoved { x: i32, y: i32 },
    PointerLeft,
    PointerReleased { x: i32, y: i32 },
    DismissRequested,
    SubmitRequested,
    Resized { width: u32, height: u32 },
    DpiChanged { dpi: u32 },
    ClockRefreshRequested,
    FocusRefreshRequested,
    RenderRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContextMenuEvent {
    PointerMoved { x: i32, y: i32 },
    PointerLeft,
    PointerReleased { x: i32, y: i32 },
    MoveSelection(SelectionDirection),
    SelectionRequested,
    DismissRequested,
    Resized { width: u32, height: u32 },
    DpiChanged { dpi: u32 },
    RenderRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwitcherEvent {
    CloseRequested,
    Resized { width: u32, height: u32 },
    DpiChanged { dpi: u32 },
    RenderRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchEdit {
    DeleteBackward,
    DeleteForward,
    MoveCursor(CursorMove),
    SelectAll,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CursorMove {
    Home,
    End,
    Previous,
    Next,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionDirection {
    Previous,
    Next,
}
