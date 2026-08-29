/// A Lotus-owned icon identity used by renderer-neutral presentation scenes.
///
/// Render backends map these semantic identifiers to their bundled resources.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum EmbeddedIcon {
    LotusPixel,
    FluentCalculator,
    FluentPower,
    FluentVolume,
    FluentNetwork,
    FluentSettings,
    FluentTray,
    FluentDismiss,
    FluentDesktop,
    FluentLock,
    FluentRestart,
    FluentSearch,
    FluentMusic,
    FluentPrevious,
    FluentPlay,
    FluentPause,
    FluentNext,
    FluentOpen,
    FluentPin,
    FluentPinOff,
}
