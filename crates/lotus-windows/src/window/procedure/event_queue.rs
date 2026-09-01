use std::cell::RefCell;
use std::collections::VecDeque;

use super::{
    ContextMenuEvent, DockContextRequest, DockEvent, PointerEvent, SearchEvent,
    SettingsEvent, StatusEvent, SwitcherEvent, WindowKind,
};

pub(in crate::window) trait QueuedEvent: Copy + private::Sealed {
    fn events(queue: &mut PendingEvents) -> &mut VecDeque<Self>;
    fn is_pointer_move(self) -> bool;
}

pub(super) struct EventQueue(RefCell<PendingEvents>);

impl EventQueue {
    pub(super) fn dock() -> Self {
        Self::new(PendingEvents::Dock(VecDeque::new()))
    }

    pub(super) fn dock_replica() -> Self {
        Self::new(PendingEvents::DockReplica(VecDeque::new()))
    }

    pub(super) fn status() -> Self {
        Self::new(PendingEvents::Status(VecDeque::new()))
    }

    pub(super) fn search() -> Self {
        Self::new(PendingEvents::Search(VecDeque::new()))
    }

    pub(super) fn settings() -> Self {
        Self::new(PendingEvents::Settings(VecDeque::new()))
    }

    pub(super) fn context_menu() -> Self {
        Self::new(PendingEvents::ContextMenu(VecDeque::new()))
    }

    pub(super) fn switcher() -> Self {
        Self::new(PendingEvents::Switcher(VecDeque::new()))
    }

    fn new(events: PendingEvents) -> Self {
        Self(RefCell::new(events))
    }

    pub(super) fn push<E: QueuedEvent>(&self, event: E) {
        let mut queue = self.0.borrow_mut();
        let events = E::events(&mut queue);
        if events
            .back()
            .is_some_and(|previous| previous.is_pointer_move())
            && event.is_pointer_move()
        {
            *events.back_mut().expect("nonempty pending queue") = event;
        } else {
            events.push_back(event);
        }
    }

    pub(super) fn drain<E: QueuedEvent>(&self) -> VecDeque<E> {
        std::mem::take(E::events(&mut self.0.borrow_mut()))
    }

    pub(super) fn push_search_context_request(&self, request: DockContextRequest) {
        let mut queue = self.0.borrow_mut();
        let PendingEvents::Search(events) = &mut *queue else {
            unreachable!("search context requests require a search window queue");
        };

        // WM_CONTEXTMENU confirms that Windows completed an in-Search right-click. A
        // dismissal queued earlier by the low-level outside-click observer belongs to the
        // same handoff and must not close the parent before its child popup opens.
        events.retain(|event| !matches!(event, SearchEvent::DismissRequested));
        events.push_back(SearchEvent::ContextMenuRequested(request));
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.borrow().is_empty()
    }

    pub(super) fn clear(&self) {
        self.0.borrow_mut().clear();
    }

    pub(super) fn kind(&self) -> WindowKind {
        self.0.borrow().kind()
    }
}

pub(in crate::window) enum PendingEvents {
    Dock(VecDeque<DockEvent>),
    DockReplica(VecDeque<DockEvent>),
    Status(VecDeque<StatusEvent>),
    Search(VecDeque<SearchEvent>),
    Settings(VecDeque<SettingsEvent>),
    ContextMenu(VecDeque<ContextMenuEvent>),
    Switcher(VecDeque<SwitcherEvent>),
}

impl PendingEvents {
    fn is_empty(&self) -> bool {
        match self {
            Self::Dock(events) | Self::DockReplica(events) => events.is_empty(),
            Self::Status(events) => events.is_empty(),
            Self::Search(events) => events.is_empty(),
            Self::Settings(events) => events.is_empty(),
            Self::ContextMenu(events) => events.is_empty(),
            Self::Switcher(events) => events.is_empty(),
        }
    }

    fn clear(&mut self) {
        match self {
            Self::Dock(events) | Self::DockReplica(events) => events.clear(),
            Self::Status(events) => events.clear(),
            Self::Search(events) => events.clear(),
            Self::Settings(events) => events.clear(),
            Self::ContextMenu(events) => events.clear(),
            Self::Switcher(events) => events.clear(),
        }
    }

    fn kind(&self) -> WindowKind {
        match self {
            Self::Dock(_) => WindowKind::Dock,
            Self::DockReplica(_) => WindowKind::DockReplica,
            Self::Status(_) => WindowKind::Status,
            Self::Search(_) => WindowKind::Search,
            Self::Settings(_) => WindowKind::Settings,
            Self::ContextMenu(_) => WindowKind::ContextMenu,
            Self::Switcher(_) => WindowKind::Switcher,
        }
    }
}

impl QueuedEvent for DockEvent {
    fn events(queue: &mut PendingEvents) -> &mut VecDeque<Self> {
        match queue {
            PendingEvents::Dock(events) | PendingEvents::DockReplica(events) => events,
            _ => unreachable!("dock events require a dock window queue"),
        }
    }

    fn is_pointer_move(self) -> bool {
        matches!(self, Self::Pointer(PointerEvent::Moved { .. }))
    }
}

impl QueuedEvent for StatusEvent {
    fn events(queue: &mut PendingEvents) -> &mut VecDeque<Self> {
        let PendingEvents::Status(events) = queue else {
            unreachable!("status events require a status window queue");
        };
        events
    }

    fn is_pointer_move(self) -> bool {
        matches!(self, Self::Pointer(PointerEvent::Moved { .. }))
    }
}

impl QueuedEvent for SearchEvent {
    fn events(queue: &mut PendingEvents) -> &mut VecDeque<Self> {
        let PendingEvents::Search(events) = queue else {
            unreachable!("search events require a search window queue");
        };
        events
    }

    fn is_pointer_move(self) -> bool {
        matches!(self, Self::PointerMoved { .. })
    }
}

impl QueuedEvent for SettingsEvent {
    fn events(queue: &mut PendingEvents) -> &mut VecDeque<Self> {
        let PendingEvents::Settings(events) = queue else {
            unreachable!("settings events require a settings window queue");
        };
        events
    }

    fn is_pointer_move(self) -> bool {
        matches!(self, Self::PointerMoved { .. })
    }
}

impl QueuedEvent for ContextMenuEvent {
    fn events(queue: &mut PendingEvents) -> &mut VecDeque<Self> {
        let PendingEvents::ContextMenu(events) = queue else {
            unreachable!("context menu events require a context menu queue");
        };
        events
    }

    fn is_pointer_move(self) -> bool {
        matches!(self, Self::PointerMoved { .. })
    }
}

impl QueuedEvent for SwitcherEvent {
    fn events(queue: &mut PendingEvents) -> &mut VecDeque<Self> {
        let PendingEvents::Switcher(events) = queue else {
            unreachable!("switcher events require a switcher window queue");
        };
        events
    }

    fn is_pointer_move(self) -> bool {
        matches!(self, Self::PointerMoved { .. })
    }
}

mod private {
    use super::{
        ContextMenuEvent, DockEvent, SearchEvent, SettingsEvent, StatusEvent, SwitcherEvent,
    };

    pub(in crate::window) trait Sealed {}

    impl Sealed for DockEvent {}
    impl Sealed for StatusEvent {}
    impl Sealed for SearchEvent {}
    impl Sealed for SettingsEvent {}
    impl Sealed for ContextMenuEvent {}
    impl Sealed for SwitcherEvent {}
}
