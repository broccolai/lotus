#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommandId {
    OpenSettings,
    OpenVolumeMixer,
    OpenNotificationArea,
    ShowDesktop,
    LockComputer,
    RestartComputer,
    ShutDownComputer,
    QuitLotus,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandEntry {
    pub id: CommandId,
    pub title: &'static str,
    keywords: &'static str,
}

const COMMANDS: [CommandEntry; 8] = [
    command(
        CommandId::OpenSettings,
        "Open Lotus Settings",
        "preferences configure",
    ),
    command(
        CommandId::OpenVolumeMixer,
        "Open Volume Mixer",
        "sound audio speaker",
    ),
    command(
        CommandId::OpenNotificationArea,
        "Open Notification Area",
        "tray hidden icons background apps",
    ),
    command(CommandId::ShowDesktop, "Show Desktop", "minimize windows"),
    command(CommandId::LockComputer, "Lock PC", "screen security"),
    command(CommandId::RestartComputer, "Restart PC", "reboot power"),
    command(CommandId::ShutDownComputer, "Shut Down PC", "power off"),
    command(CommandId::QuitLotus, "Quit Lotus", "exit close dock"),
];

const fn command(
    id: CommandId,
    title: &'static str,
    keywords: &'static str,
) -> CommandEntry {
    CommandEntry {
        id,
        title,
        keywords,
    }
}

pub fn command_query(query: &str) -> Option<&str> {
    query.trim_start().strip_prefix('>').map(str::trim)
}

pub fn matching_commands(query: &str) -> Vec<CommandEntry> {
    let terms = query
        .split_whitespace()
        .map(str::to_ascii_lowercase)
        .collect::<Vec<_>>();

    COMMANDS
        .iter()
        .copied()
        .filter(|entry| {
            let searchable =
                format!("{} {}", entry.title, entry.keywords).to_ascii_lowercase();
            terms.iter().all(|term| searchable.contains(term))
        })
        .collect()
}
