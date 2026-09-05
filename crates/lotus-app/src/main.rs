#![windows_subsystem = "windows"]

#[cfg(not(target_os = "windows"))]
compile_error!("Lotus is a native Windows application.");

mod app;

fn main() {
    lotus_windows::diagnostics::install_panic_hook();
    lotus_windows::diagnostics::record_diagnostic(
        "process.started",
        &format!(
            "executable={:?} debug_build={}",
            std::env::current_exe(),
            cfg!(debug_assertions)
        ),
    );
    match lotus_windows::update::run_helper_if_requested() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            lotus_windows::dialog::show_unowned_error(
                "Lotus Update",
                &format!("Lotus could not install the update.\n\n{error}"),
            );
            return;
        }
    }
    match lotus_windows::exclusive_taskbar::run_guardian_if_requested() {
        Ok(true) => return,
        Ok(false) => {}
        Err(error) => {
            lotus_windows::diagnostics::record_error("guardian.failed", &error);
            return;
        }
    }
    let result = app::run();
    lotus_windows::diagnostics::record_state(
        "process.app_returned",
        &[("success", u64::from(result.is_ok()))],
    );
    lotus_windows::diagnostics::record_diagnostic(
        "responsiveness",
        &lotus_windows::responsiveness::METRICS.snapshot().to_text(),
    );
    if let Err(error) = result {
        lotus_windows::dialog::show_unowned_error(
            "Lotus",
            &format!("Lotus could not continue.\n\n{error}"),
        );
    }
}
