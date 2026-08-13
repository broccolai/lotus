use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant, SystemTime};
use std::{fs, thread};

use thiserror::Error;
use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HWND, LPARAM, WAIT_OBJECT_0, WAIT_TIMEOUT, WPARAM,
};
use windows::Win32::System::Threading::{
    GetCurrentThreadId, OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::WindowsAndMessaging::{
    EVENT_OBJECT_CREATE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW, FindWindowExW,
    FindWindowW, GetClassNameW, GetMessageW, IsWindow, IsWindowVisible, MSG, OBJID_WINDOW,
    PM_NOREMOVE, PeekMessageW, PostThreadMessageW, SW_HIDE, SW_SHOWNOACTIVATE,
    ShowWindowAsync, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_APP, WM_QUIT,
};
use windows::core::{Error, PCWSTR, w};

use super::taskbar_state::{TaskbarStateError, TaskbarStateGuard};
use crate::NativeError;

const GUARDIAN_ARGUMENT: &str = "--lotus-taskbar-guardian";
const READY_FILE: &str = "ready";
const STOP_FILE: &str = "stop";
const START_TIMEOUT: Duration = Duration::from_secs(5);
const POLL_INTERVAL_MILLISECONDS: u32 = 100;
const TASKBAR_EVENT_MESSAGE: u32 = WM_APP + 0x4CA;
static EVENT_THREAD_ID: AtomicU32 = AtomicU32::new(0);

#[derive(Debug, Error)]
pub enum ExclusiveTaskbarError {
    #[error("exclusive taskbar mode could not access its recovery directory: {0}")]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    TaskbarState(#[from] TaskbarStateError),
    #[error("invalid exclusive-taskbar guardian arguments")]
    InvalidGuardianArguments,
    #[error("the exclusive-taskbar guardian stopped before it became ready")]
    GuardianStopped,
    #[error("the exclusive-taskbar guardian did not become ready within five seconds")]
    GuardianTimedOut,
    #[error("the exclusive-taskbar guardian could not monitor Lotus: {0}")]
    ParentProcess(NativeError),
    #[error("the exclusive-taskbar guardian wait failed")]
    ParentWait,
    #[error("the exclusive-taskbar event observer failed: {0}")]
    EventObserver(NativeError),
    #[error("the exclusive-taskbar event observer stopped unexpectedly")]
    EventObserverStopped,
}

pub struct ExclusiveTaskbarGuard {
    child: Child,
    control_directory: PathBuf,
}

impl ExclusiveTaskbarGuard {
    pub fn start() -> Result<Self, ExclusiveTaskbarError> {
        let control_directory = control_directory();
        fs::create_dir(&control_directory)?;
        let mut child = Command::new(std::env::current_exe()?)
            .arg(GUARDIAN_ARGUMENT)
            .arg(std::process::id().to_string())
            .arg(&control_directory)
            .spawn()?;

        let started = Instant::now();
        loop {
            if control_directory.join(READY_FILE).is_file() {
                return Ok(Self {
                    child,
                    control_directory,
                });
            }
            if child.try_wait()?.is_some() {
                cleanup_control_directory(&control_directory);
                return Err(ExclusiveTaskbarError::GuardianStopped);
            }
            if started.elapsed() >= START_TIMEOUT {
                let _ = fs::write(control_directory.join(STOP_FILE), []);
                let _ = child.wait();
                cleanup_control_directory(&control_directory);
                return Err(ExclusiveTaskbarError::GuardianTimedOut);
            }
            thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for ExclusiveTaskbarGuard {
    fn drop(&mut self) {
        let _ = fs::write(self.control_directory.join(STOP_FILE), []);
        let _ = self.child.wait();
        cleanup_control_directory(&self.control_directory);
    }
}

pub fn run_guardian_if_requested() -> bool {
    let Ok(request) = guardian_request(std::env::args_os().skip(1)) else {
        return true;
    };
    let Some((parent_process_id, control_directory)) = request else {
        return false;
    };
    let _ = run_guardian(parent_process_id, &control_directory);
    true
}

fn guardian_request<I, S>(
    arguments: I,
) -> Result<Option<(u32, PathBuf)>, ExclusiveTaskbarError>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut arguments = arguments.into_iter().map(Into::into);
    let Some(first) = arguments.next() else {
        return Ok(None);
    };
    if !argument_eq(&first, GUARDIAN_ARGUMENT) {
        return Ok(None);
    }
    let process_id = arguments
        .next()
        .and_then(|value| value.to_str().and_then(|value| value.parse::<u32>().ok()))
        .filter(|value| *value != 0)
        .ok_or(ExclusiveTaskbarError::InvalidGuardianArguments)?;
    let directory = arguments
        .next()
        .map(PathBuf::from)
        .ok_or(ExclusiveTaskbarError::InvalidGuardianArguments)?;
    if arguments.next().is_some() {
        return Err(ExclusiveTaskbarError::InvalidGuardianArguments);
    }
    Ok(Some((process_id, directory)))
}

fn run_guardian(
    parent_process_id: u32,
    control_directory: &Path,
) -> Result<(), ExclusiveTaskbarError> {
    let parent = ProcessHandle::open(parent_process_id)?;
    let mut taskbar_state = TaskbarStateGuard::enable_autohide()?;
    let event_observer = TaskbarEventObserver::start()?;
    fs::write(control_directory.join(READY_FILE), [])?;

    loop {
        // SAFETY: `parent` owns a live synchronization handle and the bounded timeout
        // keeps the guardian responsive to cancellation and taskbar recreation.
        match unsafe { WaitForSingleObject(parent.0, POLL_INTERVAL_MILLISECONDS) } {
            WAIT_OBJECT_0 => break,
            WAIT_TIMEOUT => {
                if control_directory.join(STOP_FILE).exists() {
                    break;
                }
                if event_observer.is_finished() {
                    return Err(ExclusiveTaskbarError::EventObserverStopped);
                }
            }
            _ => return Err(ExclusiveTaskbarError::ParentWait),
        }
    }

    drop(event_observer);
    let _ = taskbar_state.restore();
    cleanup_control_directory(control_directory);
    Ok(())
}

struct TaskbarEventObserver {
    thread_id: u32,
    thread: Option<thread::JoinHandle<()>>,
    stop: Arc<AtomicBool>,
}

impl TaskbarEventObserver {
    fn start() -> Result<Self, ExclusiveTaskbarError> {
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("lotus-taskbar-events".into())
            .spawn(move || taskbar_event_loop(&ready_tx, &thread_stop))?;
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(thread_id)) => Ok(Self {
                thread_id,
                thread: Some(thread),
                stop,
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(ExclusiveTaskbarError::EventObserver(error.into()))
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                let thread_id = EVENT_THREAD_ID.load(Ordering::Acquire);
                if thread_id != 0 {
                    // SAFETY: A nonzero published ID belongs to the event thread's live queue.
                    let _ = unsafe {
                        PostThreadMessageW(thread_id, WM_QUIT, WPARAM(0), LPARAM(0))
                    };
                }
                let _ = thread.join();
                Err(ExclusiveTaskbarError::EventObserverStopped)
            }
        }
    }

    fn is_finished(&self) -> bool {
        self.thread
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
    }
}

impl Drop for TaskbarEventObserver {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // SAFETY: `thread_id` was published only after the observer created its message queue.
        let _ =
            unsafe { PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0)) };
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn taskbar_event_loop(ready: &mpsc::SyncSender<Result<u32, Error>>, stop: &AtomicBool) {
    let mut message = MSG::default();
    // SAFETY: A no-remove peek is the documented way to establish this thread's message queue.
    let _ = unsafe { PeekMessageW(&raw mut message, None, 0, 0, PM_NOREMOVE) };
    // SAFETY: This call only identifies the current event-loop thread.
    let thread_id = unsafe { GetCurrentThreadId() };
    EVENT_THREAD_ID.store(thread_id, Ordering::Release);

    let hooks = [
        EVENT_OBJECT_CREATE,
        EVENT_OBJECT_SHOW,
        EVENT_OBJECT_LOCATIONCHANGE,
    ]
    .into_iter()
    .map(OwnedWinEventHook::install)
    .collect::<Result<Vec<_>, _>>();
    let hooks = match hooks {
        Ok(hooks) => hooks,
        Err(error) => {
            EVENT_THREAD_ID.store(0, Ordering::Release);
            let _ = ready.send(Err(error));
            return;
        }
    };

    let mut windows = TaskbarWindows::default();
    if stop.load(Ordering::Acquire) {
        EVENT_THREAD_ID.store(0, Ordering::Release);
        return;
    }
    windows.hide_visible();
    if ready.send(Ok(thread_id)).is_err() {
        EVENT_THREAD_ID.store(0, Ordering::Release);
        return;
    }

    loop {
        // SAFETY: `message` remains writable and this thread owns its message queue.
        let result = unsafe { GetMessageW(&raw mut message, None, 0, 0) }.0;
        if result <= 0 {
            break;
        }
        if message.message == TASKBAR_EVENT_MESSAGE {
            let hwnd = HWND(std::ptr::with_exposed_provenance_mut(message.wParam.0));
            windows.hide_window(hwnd);
        }
    }

    drop(hooks);
    EVENT_THREAD_ID.store(0, Ordering::Release);
}

struct OwnedWinEventHook(HWINEVENTHOOK);

impl OwnedWinEventHook {
    fn install(event: u32) -> Result<Self, Error> {
        // SAFETY: The static callback contains no borrowed state. Out-of-context delivery keeps
        // Lotus code outside Explorer, and this process owns a pumping message queue.
        let hook = unsafe {
            SetWinEventHook(
                event,
                event,
                None,
                Some(taskbar_event_callback),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if hook.is_invalid() {
            Err(Error::from_thread())
        } else {
            Ok(Self(hook))
        }
    }
}

impl Drop for OwnedWinEventHook {
    fn drop(&mut self) {
        // SAFETY: This guard owns the successful event-hook registration exactly once.
        let _ = unsafe { UnhookWinEvent(self.0) };
    }
}

unsafe extern "system" fn taskbar_event_callback(
    _hook: HWINEVENTHOOK,
    _event: u32,
    hwnd: HWND,
    object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    if hwnd.0.is_null() || object_id != OBJID_WINDOW.0 || !is_taskbar_window(hwnd) {
        return;
    }
    let thread_id = EVENT_THREAD_ID.load(Ordering::Acquire);
    if thread_id == 0 {
        return;
    }
    // SAFETY: The observer thread publishes its ID after creating a message queue. The HWND is
    // an opaque integer value and contains no borrowed process memory.
    let _ = unsafe {
        PostThreadMessageW(
            thread_id,
            TASKBAR_EVENT_MESSAGE,
            WPARAM(hwnd.0.addr()),
            LPARAM(0),
        )
    };
}

#[derive(Default)]
struct TaskbarWindows {
    entries: Vec<TaskbarWindow>,
}

impl Drop for TaskbarWindows {
    fn drop(&mut self) {
        self.restore();
    }
}

struct TaskbarWindow {
    hwnd: HWND,
    was_visible: bool,
}

impl TaskbarWindows {
    fn hide_visible(&mut self) {
        for hwnd in taskbar_windows() {
            self.hide_window(hwnd);
        }
    }

    fn hide_window(&mut self, hwnd: HWND) {
        // SAFETY: The callback or current shell lookup supplied a live top-level taskbar HWND.
        if !unsafe { IsWindowVisible(hwnd).as_bool() } {
            return;
        }
        let address = hwnd.0.addr();
        if !self
            .entries
            .iter()
            .any(|entry| entry.hwnd.0.addr() == address)
        {
            self.entries.push(TaskbarWindow {
                hwnd,
                was_visible: true,
            });
        }
        // SAFETY: Hiding an exact taskbar-class HWND is reversible and its visibility is journaled.
        let _ = unsafe { ShowWindowAsync(hwnd, SW_HIDE) };
    }

    fn restore(&self) {
        for entry in &self.entries {
            // SAFETY: The handle is only used when Windows still recognizes it.
            if entry.was_visible && unsafe { IsWindow(Some(entry.hwnd)).as_bool() } {
                // SAFETY: This restores only a taskbar window that was visible when
                // the guardian first observed it, without activating it.
                let _ = unsafe { ShowWindowAsync(entry.hwnd, SW_SHOWNOACTIVATE) };
            }
        }
    }
}

fn is_taskbar_window(hwnd: HWND) -> bool {
    let mut class_name = [0u16; 32];
    // SAFETY: `class_name` is writable for the duration of this synchronous query.
    let length = unsafe { GetClassNameW(hwnd, &mut class_name) };
    let Ok(length) = usize::try_from(length) else {
        return false;
    };
    matches!(
        String::from_utf16_lossy(&class_name[..length]).as_str(),
        "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
    )
}

fn taskbar_windows() -> Vec<HWND> {
    let mut windows = Vec::new();
    // SAFETY: Static class strings are NUL-terminated and a null title accepts any title.
    if let Ok(primary) = unsafe { FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()) } {
        windows.push(primary);
    }
    let mut previous = None;
    loop {
        // SAFETY: The search is restricted to top-level windows of the exact secondary
        // taskbar class; `previous` is either null or a handle returned by this loop.
        let Ok(hwnd) = (unsafe {
            FindWindowExW(None, previous, w!("Shell_SecondaryTrayWnd"), PCWSTR::null())
        }) else {
            break;
        };
        windows.push(hwnd);
        previous = Some(hwnd);
    }
    windows
}

struct ProcessHandle(HANDLE);

impl ProcessHandle {
    fn open(process_id: u32) -> Result<Self, ExclusiveTaskbarError> {
        // SAFETY: The identifier is validated as nonzero and the requested right permits
        // waiting only; ownership of the returned handle transfers to this guard.
        unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, process_id) }
            .map(Self)
            .map_err(|error| ExclusiveTaskbarError::ParentProcess(error.into()))
    }
}

impl Drop for ProcessHandle {
    fn drop(&mut self) {
        // SAFETY: This guard owns the successful `OpenProcess` result exactly once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

fn argument_eq(argument: &OsStr, expected: &str) -> bool {
    argument
        .to_str()
        .is_some_and(|value| value.eq_ignore_ascii_case(expected))
}

fn control_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!("lotus-taskbar-{}-{nonce}", std::process::id()))
}

fn cleanup_control_directory(directory: &Path) {
    let _ = fs::remove_file(directory.join(READY_FILE));
    let _ = fs::remove_file(directory.join(STOP_FILE));
    let _ = fs::remove_dir(directory);
}
