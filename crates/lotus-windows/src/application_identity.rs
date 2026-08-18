use std::ffi::{OsStr, c_void};
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use lotus_core::window::WindowId;
use windows::Win32::Foundation::{HLOCAL, HWND, LocalFree, PROPERTYKEY};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IPersistFile, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::{
    IPropertyStore, SHGetPropertyStoreForWindow,
};
use windows::Win32::UI::Shell::{CommandLineToArgvW, IShellLinkW, ShellLink};
use windows::Win32::UI::WindowsAndMessaging::IsWindow;
use windows::core::{BSTR, GUID, Interface, PCWSTR};

use crate::launch::ComApartment;

const APP_USER_MODEL_FORMAT: GUID = GUID::from_u128(0x9f4c2855_9f79_4b39_a8d0_e1d42de1d5f3);
const APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: APP_USER_MODEL_FORMAT,
    pid: 5,
};
const RELAUNCH_COMMAND: PROPERTYKEY = PROPERTYKEY {
    fmtid: APP_USER_MODEL_FORMAT,
    pid: 2,
};
const RELAUNCH_ICON: PROPERTYKEY = PROPERTYKEY {
    fmtid: APP_USER_MODEL_FORMAT,
    pid: 3,
};
const RELAUNCH_DISPLAY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: APP_USER_MODEL_FORMAT,
    pid: 4,
};
const PREVENT_PINNING: PROPERTYKEY = PROPERTYKEY {
    fmtid: APP_USER_MODEL_FORMAT,
    pid: 9,
};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WindowApplicationIdentity {
    pub app_user_model_id: Option<String>,
    pub relaunch_command: Option<String>,
    pub display_name: Option<String>,
    pub icon_resource: Option<String>,
    pub prevent_pinning: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelaunchApplication {
    pub target: String,
    pub arguments: Option<String>,
}

pub fn window_application_identity(window: WindowId) -> Option<WindowApplicationIdentity> {
    let _apartment = ComApartment::enter()?;
    window_application_identity_in_apartment(window)
}

pub(crate) fn window_application_identity_in_apartment(
    window: WindowId,
) -> Option<WindowApplicationIdentity> {
    let window = window_handle(window)?;

    let properties: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(window) }.ok()?;

    Some(WindowApplicationIdentity {
        app_user_model_id: string_property(&properties, &APP_USER_MODEL_ID),
        relaunch_command: string_property(&properties, &RELAUNCH_COMMAND),
        display_name: string_property(&properties, &RELAUNCH_DISPLAY_NAME),
        icon_resource: string_property(&properties, &RELAUNCH_ICON),
        prevent_pinning: bool_property(&properties, &PREVENT_PINNING).unwrap_or(false),
    })
}

pub(crate) fn shortcut_application_id(path: &Path) -> Option<String> {
    let _apartment = ComApartment::enter()?;
    let shortcut: IShellLinkW =
        unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let persist: IPersistFile = shortcut.cast().ok()?;
    let path = wide_null(path.as_os_str());
    unsafe { persist.Load(PCWSTR(path.as_ptr()), STGM_READ) }.ok()?;
    let properties: IPropertyStore = shortcut.cast().ok()?;

    string_property(&properties, &APP_USER_MODEL_ID)
}

pub fn relaunch_application(command: &str) -> Option<RelaunchApplication> {
    parse_relaunch_application(command, |candidate| {
        crate::launch::resolve_executable(candidate).is_some()
            || candidate.starts_with("shell:")
    })
}

fn parse_relaunch_application(
    command: &str,
    is_launchable: impl Fn(&str) -> bool,
) -> Option<RelaunchApplication> {
    let command = wide_null(OsStr::new(command));
    let mut argument_count = 0;
    let arguments =
        unsafe { CommandLineToArgvW(PCWSTR(command.as_ptr()), &raw mut argument_count) };
    if arguments.is_null() || argument_count <= 0 {
        return None;
    }
    let arguments = LocalArguments(arguments);
    let values = unsafe {
        std::slice::from_raw_parts(arguments.0, usize::try_from(argument_count).ok()?)
    }
    .iter()
    .map(|value| unsafe { value.to_string() }.ok())
    .collect::<Option<Vec<_>>>()?;
    let target_length =
        (1..=values.len()).find(|length| is_launchable(&values[..*length].join(" ")))?;
    let target = values[..target_length].join(" ");
    let parameters = &values[target_length..];

    Some(RelaunchApplication {
        target,
        arguments: (!parameters.is_empty()).then(|| {
            parameters
                .iter()
                .map(|argument| quote_argument(argument))
                .collect::<Vec<_>>()
                .join(" ")
        }),
    })
}

fn string_property(properties: &IPropertyStore, key: &PROPERTYKEY) -> Option<String> {
    let value = unsafe { properties.GetValue(key) }.ok()?;
    let value = BSTR::try_from(&value).ok()?.to_string();
    let value = value.trim();

    (!value.is_empty()).then(|| value.to_owned())
}

fn bool_property(properties: &IPropertyStore, key: &PROPERTYKEY) -> Option<bool> {
    let value = unsafe { properties.GetValue(key) }.ok()?;
    bool::try_from(&value).ok()
}

fn window_handle(window: WindowId) -> Option<HWND> {
    let address = usize::try_from(window.get()).ok()?;
    if address == 0 {
        return None;
    }

    let window = HWND(std::ptr::with_exposed_provenance_mut::<c_void>(address));
    unsafe { IsWindow(Some(window)) }
        .as_bool()
        .then_some(window)
}

fn wide_null(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(std::iter::once(0)).collect()
}

fn quote_argument(value: &str) -> String {
    if !value.is_empty()
        && !value
            .chars()
            .any(|character| character.is_whitespace() || character == '"')
    {
        return value.to_owned();
    }

    let mut quoted = String::from("\"");
    let mut backslashes = 0;
    for character in value.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(character);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

struct LocalArguments(*mut windows::core::PWSTR);

impl Drop for LocalArguments {
    fn drop(&mut self) {
        let _ = unsafe { LocalFree(Some(HLOCAL(self.0.cast::<c_void>()))) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relaunch_contract_preserves_target_and_arguments() {
        let cases = [
            (
                r"C:\Apps\Lotus.exe --new-window",
                "C:\\Apps\\Lotus.exe",
                Some("--new-window"),
            ),
            (
                r#""C:\Program Files\Lotus\Lotus.exe" --profile "Personal Apps""#,
                "C:\\Program Files\\Lotus\\Lotus.exe",
                Some(r#"--profile "Personal Apps""#),
            ),
        ];

        for (command, target, arguments) in cases {
            let relaunch = parse_relaunch_application(command, |candidate| {
                candidate.ends_with("Lotus.exe")
            })
            .expect("valid relaunch command");
            assert_eq!(relaunch.target, target);
            assert_eq!(relaunch.arguments.as_deref(), arguments);
        }

        let steam = parse_relaunch_application(
            r"C:\Program Files (x86)\Steam\steam.exe -silent",
            |candidate| candidate.ends_with(r"Steam\steam.exe"),
        )
        .expect("unquoted executable path");
        assert_eq!(steam.target, r"C:\Program Files (x86)\Steam\steam.exe");
        assert_eq!(steam.arguments.as_deref(), Some("-silent"));
    }
}
