use std::mem::size_of;

use windows::Win32::Foundation::{CloseHandle, E_FAIL, ERROR_NO_MORE_FILES};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetProcessHandleCount,
};
use windows::core::{Error, HRESULT};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProcessResourceSample {
    pub success: bool,
    pub working_set_bytes: u64,
    pub private_bytes: u64,
    pub handle_count: u32,
    pub thread_count: u32,
}

pub(crate) fn current_process_resources() -> ProcessResourceSample {
    sample_current_process().unwrap_or_default()
}

fn sample_current_process() -> Result<ProcessResourceSample, Error> {
    let process = unsafe { GetCurrentProcess() };
    let process_id = unsafe { GetCurrentProcessId() };
    let memory_counter_size = structure_size::<PROCESS_MEMORY_COUNTERS_EX>()?;
    let mut memory = PROCESS_MEMORY_COUNTERS_EX {
        cb: memory_counter_size,
        ..Default::default()
    };
    let counters = std::ptr::from_mut(&mut memory).cast();
    unsafe { K32GetProcessMemoryInfo(process, counters, memory.cb).ok()? };

    let mut handle_count = 0;
    unsafe { GetProcessHandleCount(process, &raw mut handle_count)? };
    let thread_count =
        current_process_thread_count(process_id, structure_size::<THREADENTRY32>()?)?;

    Ok(ProcessResourceSample {
        success: true,
        working_set_bytes: memory.WorkingSetSize as u64,
        private_bytes: memory.PrivateUsage as u64,
        handle_count,
        thread_count,
    })
}

fn current_process_thread_count(process_id: u32, entry_size: u32) -> Result<u32, Error> {
    let snapshot =
        OwnedSnapshot(unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0)? });
    count_process_threads(snapshot.0, process_id, entry_size)
}

fn count_process_threads(
    snapshot: windows::Win32::Foundation::HANDLE,
    process_id: u32,
    entry_size: u32,
) -> Result<u32, Error> {
    let mut entry = THREADENTRY32 {
        dwSize: entry_size,
        ..Default::default()
    };
    unsafe { Thread32First(snapshot, &raw mut entry)? };

    let mut count = 0_u32;
    loop {
        count = count.saturating_add(u32::from(entry.th32OwnerProcessID == process_id));
        entry.dwSize = entry_size;
        match unsafe { Thread32Next(snapshot, &raw mut entry) } {
            Ok(()) => {}
            Err(error) if error.code() == HRESULT::from_win32(ERROR_NO_MORE_FILES.0) => {
                return Ok(count);
            }
            Err(error) => return Err(error),
        }
    }
}

fn structure_size<T>() -> Result<u32, Error> {
    u32::try_from(size_of::<T>())
        .map_err(|_| Error::new(E_FAIL, "Lotus structure is too large"))
}

struct OwnedSnapshot(windows::Win32::Foundation::HANDLE);

impl Drop for OwnedSnapshot {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.0) };
    }
}
