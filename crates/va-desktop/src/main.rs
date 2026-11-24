//! Vibe Assembly desktop - a native Zed-style workbench.
//!
//! The workbench reads the real `.ai/` workspace, the RepoMap, the immutable
//! session DAG, compiler diagnostics, and the eight harness surfaces. It uses
//! GPUI directly: no web view, no npm runtime, no Electron shell.

mod assistant_panel;
mod bottom_drawer;
mod branding;
mod command_palette;
mod design;
mod editor_pane;
mod left_dock;
mod spell;
mod state;
mod theme;
mod ui_kit;
mod widgets;

use std::path::PathBuf;

use gpui::prelude::*;
use gpui::{
    px, size, App, Application, TitlebarOptions, WindowBackgroundAppearance, WindowBounds,
    WindowOptions,
};
use va_core::modes::{ExecutionMode, LanguageLens};
use va_core::state::Workspace;

use crate::state::{Workbench, WorkbenchState};

#[derive(Debug, Default, PartialEq, Eq)]
struct DesktopArgs {
    start: Option<PathBuf>,
    sample: Option<Sample>,
    workbench: bool,
    mode: Option<String>,
    lang: Option<String>,
    model: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
enum DesktopAction {
    Launch(DesktopArgs),
    Version,
    Help,
}

const DESKTOP_HELP: &str = "Vibe Assembly desktop

Usage:
  vibe-assembly-desktop [OPTIONS] [ROOT]

Options:
      --design-preview             Open the Obsidian design preview
      --design-preview-friendly    Open the Paper & Ink preview
      --design-preview-midnight    Open the Midnight preview
      --design-preview-studio      Open the Studio preview
      --design-preview-glass       Open the Liquid Glass preview
      --workbench                  Open the workspace editor workbench
      --root <PATH>                Workspace root (discovered upward)
      --mode <MODE>                Initial mode: code, web, chat, debug
      --lang <LANG>                Initial language: auto, rust, c, python, ts, go
      --model <MODEL>              Initial router model or combo
  -h, --help                       Print this help and exit
  -V, --version                    Print version and exit";

fn option_value(args: &mut impl Iterator<Item = String>, option: &str) -> Result<String, String> {
    let Some(value) = args.next() else {
        return Err(format!("{option} requires a value"));
    };
    if value.starts_with('-') || value.is_empty() {
        return Err(format!("{option} requires a value"));
    }
    Ok(value)
}

fn parse_desktop_args(args: impl IntoIterator<Item = String>) -> Result<DesktopAction, String> {
    let mut parsed = DesktopArgs::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-V" | "--version" => return Ok(DesktopAction::Version),
            "-h" | "--help" => return Ok(DesktopAction::Help),
            "--design-preview" => parsed.sample = Some(Sample::Obsidian),
            "--design-preview-friendly" => parsed.sample = Some(Sample::Paper),
            "--design-preview-midnight" => parsed.sample = Some(Sample::Midnight),
            "--design-preview-studio" => parsed.sample = Some(Sample::Studio),
            "--design-preview-glass" => parsed.sample = Some(Sample::Glass),
            "--workbench" => parsed.workbench = true,
            "--root" => {
                let value = option_value(&mut args, "--root")?;
                parsed.start = Some(PathBuf::from(value));
            }
            "--mode" => {
                parsed.mode = Some(option_value(&mut args, "--mode")?);
            }
            "--lang" => {
                parsed.lang = Some(option_value(&mut args, "--lang")?);
            }
            "--model" => {
                parsed.model = Some(option_value(&mut args, "--model")?);
            }
            other if other.starts_with('-') => return Err(format!("unknown option '{other}'")),
            other => parsed.start = Some(PathBuf::from(other)),
        }
    }
    Ok(DesktopAction::Launch(parsed))
}

fn main() {
    let args = parse_desktop_args(std::env::args().skip(1)).unwrap_or_else(|error| {
        eprintln!("vibe-assembly-desktop: {error}\nTry '--help' for usage.");
        std::process::exit(2);
    });
    let args = match args {
        DesktopAction::Version => {
            println!("vibe-assembly-desktop {}", env!("CARGO_PKG_VERSION"));
            return;
        }
        DesktopAction::Help => {
            println!("{DESKTOP_HELP}");
            return;
        }
        DesktopAction::Launch(args) => args,
    };

    if let Some(mode) = args.mode {
        std::env::set_var("VIBE_MODE", mode);
    }
    if let Some(lang) = args.lang {
        std::env::set_var("VIBE_LANG", lang);
    }
    if let Some(model) = args.model {
        std::env::set_var("VIBE_MODEL", model);
    }

    let mode = std::env::var("VIBE_MODE")
        .ok()
        .and_then(|value| value.parse::<ExecutionMode>().ok())
        .unwrap_or(ExecutionMode::Code);

    if let Some(sample) = args.sample {
        run_preview(mode, sample);
        return;
    }

    // A bare launch opens the Liquid Glass surface; the workspace workbench
    // needs an explicit `--workbench` or a root path because it requires a repo.
    if !args.workbench && args.start.is_none() {
        run_preview_shipped(mode);
        return;
    }

    let language = std::env::var("VIBE_LANG")
        .ok()
        .and_then(|value| value.parse::<LanguageLens>().ok())
        .unwrap_or(LanguageLens::Auto);
    let model = std::env::var("VIBE_MODEL").unwrap_or_else(|_| "auto-combo".to_string());

    let start = args
        .start
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    match Workspace::discover(&start) {
        Some(root) => run(Workspace::load(&root), None, mode, language, model),
        None => {
            let shown = start.display().to_string();
            run(
                Workspace::load(&start),
                Some(format!("Searched upward from: {shown}")),
                mode,
                language,
                model,
            );
        }
    }
}

fn run(
    workspace: Workspace,
    error: Option<String>,
    mode: ExecutionMode,
    language: LanguageLens,
    model: String,
) {
    Application::new()
        .with_assets(branding::BrandAssets)
        .run(move |cx: &mut App| {
            theme::init_fonts(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1440.0), px(900.0)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Vibe Assembly".into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(px(1120.0), px(720.0))),
                window_background: WindowBackgroundAppearance::Opaque,
                app_id: Some("vibe-assembly".to_string()),
                ..Default::default()
            };

            let _window = cx
                .open_window(options, move |window, cx| {
                    let state = WorkbenchState::new(workspace, error, mode, language, model);
                    let view = cx.new(|cx| Workbench::new(state, cx));
                    let focus = view.read(cx).composer_focus.clone();
                    focus.focus(window);
                    view
                })
                .expect("failed to open the Vibe Assembly workbench window");

            cx.activate(true);
        });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sample {
    Obsidian,
    Paper,
    Midnight,
    Studio,
    Glass,
}

fn run_preview_shipped(mode: ExecutionMode) {
    run_preview_window(mode, Sample::Glass, "Vibe Assembly");
}

fn run_preview(mode: ExecutionMode, sample: Sample) {
    let title = match sample {
        Sample::Obsidian => "Vibe Assembly - Design Preview",
        Sample::Paper => "Vibe Assembly - Paper & Ink",
        Sample::Midnight => "Vibe Assembly - Midnight Paper",
        Sample::Studio => "Vibe Assembly - Studio",
        Sample::Glass => "Vibe Assembly - Liquid Glass",
    };
    run_preview_window(mode, sample, title);
}

fn run_preview_window(mode: ExecutionMode, sample: Sample, title: &'static str) {
    Application::new()
        .with_assets(branding::BrandAssets)
        .run(move |cx: &mut App| {
            theme::init_fonts(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1440.0), px(900.0)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some(title.into()),
                    appears_transparent: true,
                    traffic_light_position: None,
                }),
                window_min_size: Some(size(px(1120.0), px(720.0))),
                window_background: WindowBackgroundAppearance::Opaque,
                app_id: Some("vibe-assembly".to_string()),
                ..Default::default()
            };

            match sample {
                Sample::Obsidian => {
                    cx.open_window(options, move |_window, cx| {
                        cx.new(|_| design::preview::DesignPreview::new(mode))
                    })
                    .expect("failed to open the design preview window");
                }
                Sample::Paper => {
                    cx.open_window(options, move |_window, cx| {
                        cx.new(|_| design::friendly::FriendlyPreview::new(mode))
                    })
                    .expect("failed to open the design preview window");
                }
                Sample::Midnight => {
                    cx.open_window(options, move |_window, cx| {
                        cx.new(|_| design::midnight::MidnightPreview::new(mode))
                    })
                    .expect("failed to open the design preview window");
                }
                Sample::Studio => {
                    cx.open_window(options, move |_window, cx| {
                        cx.new(|_| design::studio::StudioPreview::new(mode))
                    })
                    .expect("failed to open the design preview window");
                }
                Sample::Glass => {
                    cx.open_window(options, move |_window, cx| {
                        cx.new(|cx| design::glass::GlassPreview::new(mode, cx))
                    })
                    .expect("failed to open the design preview window");
                }
            }

            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_string()).collect()
    }

    #[test]
    fn desktop_version_flags_exit_before_launch() {
        for flag in ["--version", "-V"] {
            assert!(matches!(
                parse_desktop_args(args(&[flag])),
                Ok(DesktopAction::Version)
            ));
        }
    }

    #[test]
    fn desktop_help_flags_exit_before_launch() {
        for flag in ["--help", "-h"] {
            assert!(matches!(
                parse_desktop_args(args(&[flag])),
                Ok(DesktopAction::Help)
            ));
        }
        for option in [
            "--design-preview",
            "--design-preview-friendly",
            "--design-preview-midnight",
            "--design-preview-studio",
            "--design-preview-glass",
            "--workbench",
            "--root",
            "--mode",
            "--lang",
            "--model",
        ] {
            assert!(DESKTOP_HELP.contains(option), "missing help for {option}");
        }
    }

    #[test]
    fn desktop_workbench_root_and_positional_root_parse() {
        let Ok(DesktopAction::Launch(parsed)) =
            parse_desktop_args(args(&["--workbench", "--root", "."]))
        else {
            panic!("workbench must launch");
        };
        assert!(parsed.workbench);
        assert_eq!(parsed.start.as_deref(), Some(Path::new(".")));

        let Ok(DesktopAction::Launch(parsed)) = parse_desktop_args(args(&["workspace"])) else {
            panic!("positional root must launch");
        };
        assert_eq!(parsed.start.as_deref(), Some(Path::new("workspace")));
        assert!(!parsed.workbench);

        let Ok(DesktopAction::Launch(parsed)) = parse_desktop_args(args(&[
            "--mode",
            "debug",
            "--lang",
            "rust",
            "--model",
            "daily-free",
        ])) else {
            panic!("selectors must launch");
        };
        assert_eq!(parsed.mode.as_deref(), Some("debug"));
        assert_eq!(parsed.lang.as_deref(), Some("rust"));
        assert_eq!(parsed.model.as_deref(), Some("daily-free"));
    }

    #[test]
    fn desktop_rejects_unknown_flags_and_missing_values() {
        assert_eq!(
            parse_desktop_args(args(&["--unknown"])),
            Err("unknown option '--unknown'".to_string())
        );
        for flag in ["--root", "--mode", "--lang", "--model"] {
            assert_eq!(
                parse_desktop_args(args(&[flag])),
                Err(format!("{flag} requires a value"))
            );
        }
    }
}
