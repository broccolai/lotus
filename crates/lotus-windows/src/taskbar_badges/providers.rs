use windows::Win32::Foundation::HWND;
use windows::Win32::UI::Accessibility::{IUIAutomation, TreeScope_Descendants};

use super::discovery;

pub fn badge_count(automation_id: &str, description: &str) -> Option<u32> {
    if description.eq_ignore_ascii_case("no notifications") || description.trim().is_empty()
    {
        return None;
    }
    let number = description
        .split(|character: char| !character.is_ascii_digit())
        .find(|part| !part.is_empty())
        .and_then(|part| part.parse::<u32>().ok());
    match number {
        Some(_) if is_discord(automation_id) => Some(10),
        Some(0) => None,
        Some(value) => Some(value),
        None => Some(1),
    }
}

pub fn discord_badge_count(automation: &IUIAutomation, automation_id: &str) -> Option<u32> {
    let executable_name = match automation_id {
        "Appid: com.squirrel.Discord.Discord" => "Discord.exe",
        "Appid: com.squirrel.DiscordCanary.DiscordCanary" => "DiscordCanary.exe",
        _ => return None,
    };
    discovery::discord_windows(executable_name)
        .into_iter()
        .find_map(|window| discord_window_badge_count(automation, window))
}

fn discord_window_badge_count(automation: &IUIAutomation, window: HWND) -> Option<u32> {
    // SAFETY: `window` is a live top-level HWND found synchronously above.
    let root = unsafe { automation.ElementFromHandle(window) }.ok()?;
    // SAFETY: The condition and returned elements belong to this automation client.
    let condition = unsafe { automation.CreateTrueCondition() }.ok()?;
    // SAFETY: The query is synchronous and all COM values remain alive while traversed.
    let elements = unsafe { root.FindAll(TreeScope_Descendants, &condition) }.ok()?;
    // SAFETY: The walker belongs to this automation client.
    let walker = unsafe { automation.RawViewWalker() }.ok()?;
    // SAFETY: The returned array stays alive for all bounded indexed reads below.
    let length = unsafe { elements.Length() }.ok()?;
    for index in 0..length {
        // SAFETY: `index` is bounded by the array length returned above.
        let Ok(element) = (unsafe { elements.GetElement(index) }) else {
            continue;
        };
        // SAFETY: UI Automation owns the returned BSTR and windows-rs copies it safely.
        let Ok(value) = (unsafe { element.CurrentName() }) else {
            continue;
        };
        let value = value.to_string();
        let Ok(count) = value.parse::<u32>() else {
            continue;
        };
        // SAFETY: `element` belongs to this tree and the parent query is read-only.
        let Ok(parent) = (unsafe { walker.GetParentElement(&element) }) else {
            continue;
        };
        // SAFETY: UI Automation owns the returned BSTR and windows-rs copies it safely.
        let Ok(parent_class) = (unsafe { parent.CurrentClassName() }) else {
            continue;
        };
        let parent_class = parent_class.to_string();
        if parent_class.starts_with("lowerBadge_") {
            return Some(count);
        }
    }
    None
}

pub fn is_discord(automation_id: &str) -> bool {
    automation_id.starts_with("Appid: com.squirrel.Discord")
}

pub fn is_supported_application(automation_id: &str, name: &str) -> bool {
    is_discord(automation_id)
        || automation_id.to_ascii_lowercase().contains("slack")
        || taskbar_display_name(name).eq_ignore_ascii_case("Slack")
}

pub fn taskbar_display_name(name: &str) -> &str {
    name.rsplit_once(" - ")
        .filter(|(_, suffix)| {
            suffix.ends_with("running window") || suffix.ends_with("running windows")
        })
        .map_or(name, |(display_name, _)| display_name)
}
