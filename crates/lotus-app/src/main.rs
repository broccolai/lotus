#![windows_subsystem = "windows"]

#[cfg(not(target_os = "windows"))]
compile_error!("Lotus is a native Windows application.");

mod app;

fn main() {
    lotus_windows::diagnostics::install_panic_hook();
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
    if lotus_windows::exclusive_taskbar::run_guardian_if_requested() {
        return;
    }
    let result = app::run();
    lotus_windows::diagnostics::record_diagnostic(
        "responsiveness",
        &lotus_windows::responsiveness::METRICS.snapshot().to_text(),
    );
    if let Err(error) = result {
        lotus_windows::dialog::show_unowned_error(
            "Lotus",
            &format!("Lotus could not start.\n\n{error}"),
        );
    }
}
