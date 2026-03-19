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
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("kubectui {}", env!("CARGO_PKG_VERSION"));
        return Ok(true);
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("KubecTUI — keyboard-driven terminal UI for Kubernetes\n");
        println!("USAGE: kubectui [OPTIONS]\n");
        println!("OPTIONS:");
        println!("  --theme <name>  Set color theme (dark, nord, dracula, catppuccin, light)");
        println!("  --profile-render  Enable render profiling (frame timings + folded stacks)");
        println!("  --profile-output <dir>  Profile output directory (default: target/profiles)");
        println!("  --version, -V   Show version");
        println!("  --help, -h      Show this help message");
        return Ok(true);
    }
    if let Some(pos) = args.iter().position(|a| a == "--theme")
        && let Some(name) = args.get(pos + 1)
    {
        let idx = match name.to_lowercase().as_str() {
            "nord" => 1,
            "dracula" => 2,
            "catppuccin" | "mocha" => 3,
            "light" => 4,
            _ => 0,
        };
        kubectui::ui::theme::set_active_theme(idx);
    }
    if args.iter().any(|a| a == "--profile-render") {
        kubectui::ui::profiling::set_enabled(true);
    }
    if let Some(pos) = args.iter().position(|a| a == "--profile-output")
        && let Some(dir) = args.get(pos + 1)
    {
        kubectui::ui::profiling::set_output_dir(PathBuf::from(dir));
    }

    Ok(false)
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
