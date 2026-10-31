//! Amber Server: Amber's phone API without the desktop app (docs/phone-api.md) - for a PC, a
//! NAS or a small Linux box, with downloadable models for recognition and cleanup.

mod config;
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
        _ => Err("not built yet".to_string()),
    };
    if let Err(e) = result {
        eprintln!("amber-server: {e}");
        std::process::exit(1);
    }
}

fn config_command(action: Option<ConfigAction>) -> Result<(), String> {
    let path = Config::path();
    let mut config = if path.exists() { Config::load(&path)? } else { Config::default() };
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
