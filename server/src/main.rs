//! Amber Server: Amber's phone API without the desktop app (docs/phone-api.md) - for a PC, a
//! NAS or a small Linux box, with downloadable models for recognition and cleanup.

mod catalog;
mod config;
mod cpu;
mod paths;
mod speech;

use clap::{Parser, Subcommand};

use config::Config;

#[derive(Parser)]
#[command(name = "amber-server", version, about = "Amber's speech-to-text API for your phone, on this machine")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Set up: hardware check, a speech model, the access key
    Setup {
        /// No questions: the recommended model and the defaults
        #[arg(long)]
        quick: bool,
    },
    /// Speech models: list, add, remove, choose the default
    Models {
        #[command(subcommand)]
        action: Option<ModelAction>,
    },
    /// Cleanup models: list, add, remove, choose the default, or switch the cleanup off
    Cleanup {
        #[command(subcommand)]
        action: Option<CleanupAction>,
    },
    /// Show every setting, or get and set one
    Config {
        #[command(subcommand)]
        action: Option<ConfigAction>,
    },
    /// Serve the API (what the background service runs)
    Run,
    /// The phone's address, with a QR code
    Url,
    /// The access key
    Key {
        /// A new key: every phone with the old address is locked out
        #[arg(long)]
        new: bool,
    },
    /// The background service
    Service {
        #[command(subcommand)]
        action: ServiceAction,
    },
    /// Config, models, service and whether the server answers
    Status,
    /// The service's last log lines
    Logs,
}

#[derive(Subcommand)]
enum ModelAction {
    /// Download a model
    Add { id: String },
    /// Delete a downloaded model
    Remove { id: String },
    /// The model a request gets that names none
    Default { id: String },
}

#[derive(Subcommand)]
enum CleanupAction {
    /// Download a cleanup model (and the cleanup engine with the first one)
    Add { id: String },
    /// Delete a downloaded cleanup model
    Remove { id: String },
    /// The cleanup model a request gets that names none
    Default { id: String },
    /// No cleanup
    Off,
}

#[derive(Subcommand)]
enum ConfigAction {
    /// One setting's value
    Get { key: String },
    /// Change a setting
    Set { key: String, value: String },
}

#[derive(Subcommand)]
enum ServiceAction {
    /// Start with the system (systemd user unit / Windows task at log-on)
    Install,
    /// Stop and remove the service
    Uninstall,
    /// Whether it runs
    Status,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Config { action } => config_command(action),
        Command::Models { action } => models_command(action),
        Command::Cleanup { action } => cleanup_command(action),
        _ => Err("not built yet".to_string()),
    };
    if let Err(e) = result {
        eprintln!("amber-server: {e}");
        std::process::exit(1);
    }
}

/// The config, or the defaults before `setup` ran.
fn current() -> Result<(std::path::PathBuf, Config), String> {
    let path = Config::path();
    let config = if path.exists() { Config::load(&path)? } else { Config::default() };
    Ok((path, config))
}

fn models_command(action: Option<ModelAction>) -> Result<(), String> {
    let (path, mut config) = current()?;
    match action {
        None => println!("{}", catalog::list_speech(&config, catalog::machine(&config))),
        Some(ModelAction::Add { id }) => {
            catalog::download_speech(&id)?;
            println!("{id} is downloaded. Make it the default: amber-server models default {id}");
        }
        Some(ModelAction::Remove { id }) => {
            let m = catalog::speech_model(&id)?;
            catalog::removable(Some(&config.speech), &id, "models")?;
            let file = paths::speech_dir().join(m.file);
            if file.exists() {
                std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            }
            println!("{id} removed.");
        }
        Some(ModelAction::Default { id }) => {
            let m = catalog::speech_model(&id)?;
            if !paths::model_ready(&paths::speech_dir(), m) {
                return Err(format!("{id} is not downloaded - amber-server models add {id}"));
            }
            config.speech = id.clone();
            config.save(&path)?;
            println!("{id} is the default now (a running server picks it up on restart).");
        }
    }
    Ok(())
}

fn cleanup_command(action: Option<CleanupAction>) -> Result<(), String> {
    let (path, mut config) = current()?;
    match action {
        None => println!("{}", catalog::list_cleanup(&config)),
        Some(CleanupAction::Add { id }) => {
            catalog::download_cleanup(&id, catalog::machine(&config).gpu)?;
            if config.cleanup.is_none() {
                config.cleanup = Some(id.clone());
                config.save(&path)?;
                println!("{id} is downloaded and the cleanup's default.");
            } else {
                println!("{id} is downloaded. Make it the default: amber-server cleanup default {id}");
            }
        }
        Some(CleanupAction::Remove { id }) => {
            let m = catalog::cleanup_model(&id)?;
            catalog::removable(config.cleanup.as_deref(), &id, "cleanup")?;
            let file = paths::cleanup_dir().join(m.file);
            if file.exists() {
                std::fs::remove_file(&file).map_err(|e| format!("{}: {e}", file.display()))?;
            }
            println!("{id} removed.");
        }
        Some(CleanupAction::Default { id }) => {
            let m = catalog::cleanup_model(&id)?;
            if !paths::llm_ready(&paths::cleanup_dir(), m) {
                return Err(format!("{id} is not downloaded - amber-server cleanup add {id}"));
            }
            config.cleanup = Some(id.clone());
            config.save(&path)?;
            println!("{id} is the cleanup's default now.");
        }
        Some(CleanupAction::Off) => {
            config.cleanup = None;
            config.save(&path)?;
            println!("The cleanup is off. Downloaded models stay; amber-server cleanup default <id> turns it back on.");
        }
    }
    Ok(())
}

fn config_command(action: Option<ConfigAction>) -> Result<(), String> {
    let (path, mut config) = current()?;
    match action {
        None => {
            println!("{}", path.display());
            for key in config::KEYS {
                println!("  {key} = {}", config.get(key)?);
            }
        }
        Some(ConfigAction::Get { key }) => println!("{}", config.get(&key)?),
        Some(ConfigAction::Set { key, value }) => {
            config.set(&key, &value)?;
            config.save(&path)?;
            println!("{key} = {}", config.get(&key)?);
        }
    }
    Ok(())
}
