use windows::Win32::System::SystemInformation::GetLocalTime;

pub fn local_time_24h() -> String {
    // SAFETY: This parameterless query returns a copied SYSTEMTIME value.
    let local = unsafe { GetLocalTime() };
    format!("{:02}:{:02}", local.wHour, local.wMinute)
}
