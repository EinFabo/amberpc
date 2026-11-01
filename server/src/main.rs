//! Amber Server: Amber's phone API without the desktop app (docs/phone-api.md) - for a PC, a
//! NAS or a small Linux box, with downloadable models for recognition and cleanup.

mod catalog;
mod child;
mod cleanup;
mod config;
mod cpu;
mod gpu;
mod info;
mod paths;
mod serve;
mod service;
mod setup;
mod tunnel;
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
    Run {
        /// Write the output to this file (what the Windows task does).
        #[arg(long, hide = true)]
        log: Option<std::path::PathBuf>,
    },
    /// The phone's address, with a QR code
    Url {
        /// The address of this key (default: the first)
        #[arg(long)]
        key: Option<String>,
    },
    /// The access keys: list them, or add, renew and remove one
    Key {
        #[command(subcommand)]
        action: Option<KeyAction>,
    },
    /// Reachable from anywhere through a Cloudflare Tunnel: its state, or set it up or off
    Tunnel {
        #[command(subcommand)]
        action: Option<TunnelAction>,
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
    /// Whether this build sees a usable GPU (the installers ask; exit 1 without one)
    #[command(hide = true)]
    GpuCheck,
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
enum KeyAction {
    /// A new key for another device, with its address
    Add {
        name: String,
        /// Only these models (ids, comma-separated); default: every model
        #[arg(long, value_delimiter = ',')]
        models: Option<Vec<String>>,
        /// Its requests a day (default 2000)
        #[arg(long)]
        per_day: Option<u32>,
    },
    /// A new secret for a key: devices with its old address are locked out
    New { name: Option<String> },
    /// Remove a key: its devices are locked out, the others go on
    Remove { name: String },
}

#[derive(Subcommand)]
enum TunnelAction {
    /// Use the tunnel made in Cloudflare (public hostname -> http://localhost:<port>); the token is read from the input
    Setup {
        /// The public hostname, e.g. amber.example.com
        hostname: Option<String>,
        /// The file tools/setup-tunnel.sh hands over (token and hostname), read and deleted
        #[arg(long)]
        from_file: Option<std::path::PathBuf>,
    },
    /// Switch the tunnel off (the token is kept)
    Off,
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
    // `amber-server url | head` ends quietly, like other command-line tools - not with a
    // panic. Not for `run`: the server must not die when a log reader goes away.
    #[cfg(unix)]
    if !matches!(cli.command, Command::Run { .. }) {
        // SAFETY: resets one signal to its default before any thread starts.
        unsafe {
            libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        }
    }
    let result = match cli.command {
        Command::Config { action } => config_command(action),
        Command::Setup { quick } => setup_command(quick),
        Command::Run { log } => match log.map(|l| service::redirect_output(&l)).transpose() {
            Ok(_) => serve::serve(),
            Err(e) => Err(e),
        },
        Command::Url { key } => url_command(key.as_deref()),
        Command::Key { action } => key_command(action),
        Command::Status => {
            print!("{}", info::status(&Config::path()));
            Ok(())
        }
        Command::Logs => info::logs(),
        Command::GpuCheck => match gpu::vulkan_devices() {
            [] => Err("no usable GPU for the Vulkan build here".into()),
            devices => {
                println!("{}", devices.join("\n"));
                Ok(())
            }
        },
        Command::Models { action } => models_command(action),
        Command::Cleanup { action } => cleanup_command(action),
        Command::Service { action } => service_command(action),
        Command::Tunnel { action } => tunnel_command(action),
    };
    if let Err(e) = result {
        eprintln!("amber-server: {e}");
        std::process::exit(1);
    }
}

fn setup_command(quick: bool) -> Result<(), String> {
    let path = Config::path();
    let config = if quick {
        setup::quick(&path)?
    } else {
        setup::ask(&path, &mut std::io::stdin().lock(), &mut std::io::stdout())?
    };
    println!("\nReady: speech model {}, cleanup {}.", config.speech, config.cleanup.as_deref().unwrap_or("off"));
    println!("Start it in the background: amber-server service install");
    println!("The phone's address:        amber-server url");
    if config.cleanup.is_none() {
        println!("A cleanup model:            amber-server cleanup");
    }
    Ok(())
}

/// The config of a set-up server - or the hint to run setup.
fn set_up() -> Result<(std::path::PathBuf, Config), String> {
    let (path, config) = current()?;
    if !config.set_up() {
        return Err("not set up yet - run: amber-server setup".into());
    }
    Ok((path, config))
}

fn service_command(action: ServiceAction) -> Result<(), String> {
    match action {
        ServiceAction::Install => {
            // The service would only exit with "set it up first".
            set_up()?;
            println!("{}", service::install()?);
        }
        ServiceAction::Uninstall => println!("{}", service::uninstall()?),
        ServiceAction::Status => println!("{}", service::state()),
    }
    Ok(())
}

fn url_command(name: Option<&str>) -> Result<(), String> {
    let (_, config) = set_up()?;
    let i = match name {
        Some(n) => config.find_key(n).ok_or_else(|| format!("no key named {n} - amber-server key"))?,
        None => 0,
    };
    print!("{}", info::describe(&config, &config.keys[i], &info::interfaces()));
    Ok(())
}

fn key_command(action: Option<KeyAction>) -> Result<(), String> {
    let (path, mut config) = set_up()?;
    let name = match action {
        None => {
            for k in &config.keys {
                println!("  {:<16} {}", k.name, info::key_terms(k));
            }
            println!("\nAddress: amber-server url --key <name>   another device: amber-server key add <name>");
            return Ok(());
        }
        Some(KeyAction::Add { name, models, per_day }) => {
            config.add_key(&name, models, per_day)?;
            name
        }
        Some(KeyAction::New { name }) => {
            let name = name.unwrap_or_else(|| config.keys[0].name.clone());
            config.renew_key(&name)?;
            name
        }
        Some(KeyAction::Remove { name }) => {
            config.remove_key(&name)?;
            config.save(&path)?;
            restarted("Removed - its devices are locked out, the others go on.");
            return Ok(());
        }
    };
    config.save(&path)?;
    restarted("Done.");
    let i = config.find_key(&name).expect("just added or renewed");
    print!("{}", info::describe(&config, &config.keys[i], &info::interfaces()));
    Ok(())
}

fn tunnel_command(action: Option<TunnelAction>) -> Result<(), String> {
    let (path, mut config) = set_up()?;
    match action {
        None => {
            let token = tunnel::load_token(&tunnel::token_path()).is_some();
            if !config.tunnel {
                println!("The tunnel is off{}.", if token { " (its token is kept)" } else { "" });
                println!("Reach this server from anywhere: make a tunnel in Cloudflare (Zero Trust -> Networks -> Tunnels), give it a public hostname pointing at http://localhost:{}, then: amber-server tunnel setup <hostname>", config.port);
                return Ok(());
            }
            let base = format!("https://{}", config.public_host);
            let answer = amber_core::engine::health(&base);
            let state = match answer.as_ref().and_then(|h| h["service"].as_str()) {
                Some("amber-server") => "answers through Cloudflare".to_string(),
                Some(other) => format!("Cloudflare reaches {other}, not this server - check the public hostname's target"),
                None => "not reachable yet - amber-server logs".to_string(),
            };
            println!("Tunnel on: {} - {state}", config.public_host);
            if !token {
                println!("No token - amber-server tunnel setup {}", config.public_host);
            }
            return Ok(());
        }
        Some(TunnelAction::Off) => {
            config.tunnel = false;
            config.save(&path)?;
            restarted("The tunnel is off.");
            return Ok(());
        }
        Some(TunnelAction::Setup { hostname, from_file }) => {
            let (token, host, new_key) = match from_file {
                Some(file) => {
                    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
                    let _ = std::fs::remove_file(&file);
                    let i = amber_core::api::parse_tunnel_import(&text)?;
                    (i.token, i.host, i.new_key)
                }
                None => {
                    let host = amber_core::api::public_host(&hostname.ok_or("the public hostname: amber-server tunnel setup amber.example.com")?);
                    if host.is_empty() {
                        return Err("that is not a hostname - like amber.example.com".into());
                    }
                    eprintln!("Paste the tunnel's token (or the whole install command Cloudflare shows) and press Enter:");
                    let mut pasted = String::new();
                    std::io::stdin().read_line(&mut pasted).map_err(|e| e.to_string())?;
                    let token = amber_core::api::tunnel_token(&pasted).ok_or("that is no tunnel token - it starts with eyJ")?;
                    (token, host, false)
                }
            };
            tunnel::ensure_cloudflared(&paths::data_dir())?;
            tunnel::save_token(&tunnel::token_path(), &token)?;
            if new_key {
                let first = config.keys[0].name.clone();
                config.renew_key(&first)?;
            }
            config.tunnel = true;
            config.public_host = host;
            config.save(&path)?;
            restarted("The tunnel is set up.");
            print!("{}", info::describe(&config, &config.keys[0], &info::interfaces()));
        }
    }
    Ok(())
}

/// The keys are read when the server starts: restart an installed service now.
fn restarted(done: &str) {
    match service::restart_if_installed() {
        Some(Ok(())) => println!("{done} The service restarted with it.\n"),
        Some(Err(e)) => println!("{done} The service did not restart ({e}); it takes the change when it does.\n"),
        None => println!("{done} A server running by hand takes it when you restart it.\n"),
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
        None => print!("{}", info::listing(&config, &path)),
        Some(ConfigAction::Get { key }) => println!("{}", config.get(&key)?),
        Some(ConfigAction::Set { key, value }) => {
            config.set(&key, &value)?;
            config.save(&path)?;
            println!("{key} = {}", config.get(&key)?);
        }
    }
    Ok(())
}
