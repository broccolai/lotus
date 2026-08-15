use windows::Win32::System::SystemInformation::GetLocalTime;

pub fn local_time(use_24_hour_time: bool) -> String {
    // SAFETY: This parameterless query returns a copied SYSTEMTIME value.
    let local = unsafe { GetLocalTime() };
    if use_24_hour_time {
        return format!("{:02}:{:02}", local.wHour, local.wMinute);
    }

    let period = if local.wHour < 12 {
        "AM"
    } else {
        "PM"
    };
    let hour = match local.wHour % 12 {
        0 => 12,
        hour => hour,
    };
    format!("{hour}:{:02} {period}", local.wMinute)
}

pub fn local_date() -> String {
    // SAFETY: This parameterless query returns a copied SYSTEMTIME value.
    let local = unsafe { GetLocalTime() };
    format!("{:02}/{:02}/{}", local.wDay, local.wMonth, local.wYear)
}
