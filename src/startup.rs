use std::path::PathBuf;

use anyhow::Result;

pub(crate) fn initialize_process() -> Result<bool> {
    init_logging();
    kubectui::ui::profiling::init_from_env();

    let args: Vec<String> = std::env::args().collect();
    if handle_cli_args(&args)? {
        return Ok(true);
    }

    install_panic_hook();
    Ok(false)
}

fn init_logging() {
    let log_path = dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("kubectui")
        .join("kubectui.log");
    if let Some(parent) = log_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        env_logger::Builder::from_default_env()
            .target(env_logger::Target::Pipe(Box::new(file)))
            .init();
    } else {
        env_logger::Builder::from_default_env()
            .filter_level(log::LevelFilter::Off)
            .init();
    }
}

fn handle_cli_args(args: &[String]) -> Result<bool> {
    let mut args = args.iter().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--version" | "-V" => {
                println!("kubectui {}", env!("CARGO_PKG_VERSION"));
                return Ok(true);
            }
            "--help" | "-h" => {
                println!("KubecTUI — keyboard-driven terminal UI for Kubernetes\n");
                println!("USAGE: kubectui [OPTIONS]\n");
                println!("OPTIONS:");
                println!(
                    "  --theme <name>  Set color theme (dark, nord, dracula, catppuccin, light)"
                );
                println!(
                    "  --profile-render  Enable render profiling (frame timings + folded stacks)"
                );
                println!(
                    "  --profile-output <dir>  Profile output directory (default: target/profiles)"
                );
                println!("  --version, -V   Show version");
                println!("  --help, -h      Show this help message");
                return Ok(true);
            }
            "--theme" => {
                let name = next_option_value(&mut args, "--theme")?;
                let idx = match name.to_lowercase().as_str() {
                    "nord" => 1,
                    "dracula" => 2,
                    "catppuccin" | "mocha" => 3,
                    "light" => 4,
                    _ => 0,
                };
                kubectui::ui::theme::set_active_theme(idx);
            }
            "--profile-render" => {
                kubectui::ui::profiling::set_enabled(true);
            }
            "--profile-output" => {
                let dir = next_option_value(&mut args, "--profile-output")?;
                kubectui::ui::profiling::set_output_dir(PathBuf::from(dir));
            }
            unknown if unknown.starts_with('-') => {
                anyhow::bail!("unknown option '{unknown}'");
            }
            unexpected => {
                anyhow::bail!("unexpected positional argument '{unexpected}'");
            }
        }
    }

    Ok(false)
}

fn next_option_value<'a>(
    args: &mut impl Iterator<Item = &'a String>,
    option: &str,
) -> Result<&'a str> {
    let Some(value) = args.next() else {
        anyhow::bail!("{option} requires a value");
    };
    if value.starts_with('-') {
        anyhow::bail!("{option} requires a value");
    }
    Ok(value)
}

fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::event::DisableMouseCapture
        );
        original_hook(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::handle_cli_args;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    #[test]
    fn cli_version_exits_before_terminal_startup() {
        assert!(handle_cli_args(&args(&["kubectui", "--version"])).expect("version exits"));
        assert!(handle_cli_args(&args(&["kubectui", "-V"])).expect("version exits"));
    }

    #[test]
    fn cli_rejects_unknown_options_before_terminal_startup() {
        let err = handle_cli_args(&args(&["kubectui", "--versoin"])).expect_err("unknown option");
        assert!(err.to_string().contains("unknown option"));
    }

    #[test]
    fn cli_rejects_missing_option_values_before_terminal_startup() {
        let err =
            handle_cli_args(&args(&["kubectui", "--profile-output"])).expect_err("missing value");
        assert!(err.to_string().contains("requires a value"));

        let err = handle_cli_args(&args(&["kubectui", "--theme", "--profile-render"]))
            .expect_err("flag is not a value");
        assert!(err.to_string().contains("requires a value"));
    }

    #[test]
    fn cli_rejects_positional_arguments_before_terminal_startup() {
        let err = handle_cli_args(&args(&["kubectui", "manifest.yaml"]))
            .expect_err("unexpected positional");
        assert!(err.to_string().contains("unexpected positional"));
    }
}
