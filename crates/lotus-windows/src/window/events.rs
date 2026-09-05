#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockEvent {
    Resized { width: u32, height: u32 },
    DpiChanged { dpi: u32 },
    PlacementRefreshRequested,
    Pointer(PointerEvent),
    ContextMenuRequested(DockContextRequest),
    AnimationFrame,
    MascotAnimationDeadline,
    StatusRefreshRequested,
    RenderRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StatusEvent {
    Resized { width: u32, height: u32 },
    DpiChanged { dpi: u32 },
    Pointer(PointerEvent),
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
    PointerCancelled,
    Scroll { direction: i32 },
    CloseRequested,
    TextInput(char),
    KeyPressed(SettingsKey),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SettingsKey {
    Escape,
    Backspace,
    Enter,
    Tab { reverse: bool },
    Left,
    Right,
    Up,
    Down,
    Space,
    Save,
    Paste,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DockContextRequest {
    Pointer {
        screen: SignedPoint,
        client: SignedPoint,
        shift_held: bool,
    },
    Keyboard {
        shift_held: bool,
    },
}

impl DockContextRequest {
    pub const fn shift_held(self) -> bool {
        match self {
            Self::Pointer { shift_held, .. } | Self::Keyboard { shift_held } => shift_held,
        }
    }
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
    ContextMenuRequested(DockContextRequest),
    DismissRequested(DismissRequest),
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
    Scroll(SelectionDirection),
    SelectionRequested,
    ShiftChanged(bool),
    DismissRequested(DismissRequest),
    Resized { width: u32, height: u32 },
    DpiChanged { dpi: u32 },
    RenderRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SwitcherEvent {
    CloseRequested,
    PointerMoved { x: i32, y: i32 },
    PointerLeft,
    PointerReleased { x: i32, y: i32 },
    Resized { width: u32, height: u32 },
    DpiChanged { dpi: u32 },
    RenderRequested,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DismissReason {
    Escape,
    OwnerActivated,
    Deactivated,
    OutsideClick,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DismissRequest {
    pub reason: DismissReason,
    pub(super) generation: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchEdit {
    DeleteBackward,
    DeletePreviousWord,
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
