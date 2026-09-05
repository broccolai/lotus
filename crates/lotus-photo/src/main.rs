mod config;
mod icons;
mod scenes;

use std::path::PathBuf;
use std::process::ExitCode;

use config::PhotoScene;
use lotus_core::settings::DockSettings;
use lotus_ui::embedded_icon::EmbeddedIcon;
use lotus_ui::presentation::Presentation;
use lotus_windows::backdrop;
use lotus_windows::graphics::{CompositionSurfaceState, GraphicsDevice, SurfaceSize};
use lotus_windows::interaction::next_message;
use lotus_windows::window::photo::{PhotoSession, PhotoWindow};

fn main() -> ExitCode {
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    match run(&arguments) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("lotus-photo: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(arguments: &[std::ffi::OsString]) -> Result<(), PhotoError> {
    let command = Command::parse(arguments)?;
    if command == Command::Help {
        print_help();
        return Ok(());
    }

    let path = command.path().expect("non-help commands have a scene path");
    let scene = PhotoScene::load(path)?;
    if matches!(command, Command::Render(_)) {
        lotus_windows::dpi::enable_per_monitor_v2()?;
    }
    let _session = PhotoSession::enter()?;
    let rendered = scenes::build(&scene)?;

    if matches!(command, Command::Validate(_)) {
        println!("valid: {}", path.display());
        return Ok(());
    }

    render(&rendered.presentation, rendered.size, scene.kind)?;
    Ok(())
}

fn render(
    presentation: &Presentation<EmbeddedIcon>,
    size: SurfaceSize,
    kind: config::SceneKind,
) -> Result<(), PhotoError> {
    let window = PhotoWindow::create(size.width(), size.height())?;
    match kind {
        config::SceneKind::Dock => {
            backdrop::apply_dock_settings(window.handle(), &DockSettings::default());
        }
        config::SceneKind::Search | config::SceneKind::Switcher => {
            backdrop::apply_popup_settings(window.handle(), &DockSettings::default());
        }
    }
    let graphics = GraphicsDevice::create()?;
    let mut surface = CompositionSurfaceState::create(&graphics, window.handle(), size)?;
    let _ = surface.render_scene(presentation, false)?;
    surface.commit()?;

    eprintln!("Lotus photo mode is open. Close it with Alt+F4.");
    window.show();
    while let Some(message) = next_message().map_err(|_| PhotoError::MessagePump)? {
        message.dispatch();
    }
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
enum Command {
    Render(PathBuf),
    Validate(PathBuf),
    Help,
}

impl Command {
    fn parse(arguments: &[std::ffi::OsString]) -> Result<Self, PhotoError> {
        match arguments {
            [argument] if argument == "--help" || argument == "-h" => Ok(Self::Help),
            [argument, path] if argument == "--validate" => {
                Ok(Self::Validate(scene_path(path)?))
            }
            [argument, path] if argument == "--scene" => {
                Ok(Self::Render(scene_path(path)?))
            }
            [path] => Ok(Self::Render(scene_path(path)?)),
            _ => Err(PhotoError::Usage),
        }
    }

    fn path(&self) -> Option<&std::path::Path> {
        match self {
            Self::Render(path) | Self::Validate(path) => Some(path),
            Self::Help => None,
        }
    }
}

fn scene_path(path: &std::ffi::OsString) -> Result<PathBuf, PhotoError> {
    let path = PathBuf::from(path);
    if path.as_os_str().is_empty() {
        return Err(PhotoError::Usage);
    }
    Ok(path)
}

fn print_help() {
    println!(
        "Usage:\n  lotus-photo <scene.json>\n  lotus-photo --scene <scene.json>\n  lotus-photo --validate <scene.json>\n\nThe default command opens a dedicated local photo-mode window. --validate parses\nand builds the renderer-neutral scene without creating a native window."
    );
}

#[derive(Debug, thiserror::Error)]
enum PhotoError {
    #[error("usage: lotus-photo [--scene] <scene.json> | --validate <scene.json>")]
    Usage,
    #[error(transparent)]
    Config(#[from] config::ConfigError),
    #[error(transparent)]
    Scene(#[from] scenes::SceneError),
    #[error(transparent)]
    Graphics(#[from] lotus_windows::graphics::GraphicsDeviceError),
    #[error(transparent)]
    Surface(#[from] lotus_windows::graphics::SurfaceError),
    #[error(transparent)]
    Native(#[from] lotus_windows::NativeError),
    #[error("native photo-mode message pump failed")]
    MessagePump,
}
