//! `hpx` — CLI for high-performance HTTP client and download engine.

#![allow(clippy::print_stdout)]
#![allow(missing_docs)]

use clap::{Parser, Subcommand};
use hpx_dl::{DownloadEngine, DownloadPriority};

/// High-performance HTTP client and download engine.
#[derive(Debug, Parser)]
#[command(
    name = "hpx",
    about = "High-performance HTTP client and download engine"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Download management commands.
    #[command(subcommand)]
    Dl(DlCommands),
}

#[derive(Debug, Subcommand)]
enum DlCommands {
    /// Add a new download.
    Add {
        /// URL to download.
        url: String,
        /// Destination file path.
        #[arg(short, long)]
        output: Option<String>,
        /// Priority (low, normal, high, critical).
        #[arg(short, long, default_value = "normal")]
        priority: String,
    },
    /// Pause a download.
    Pause {
        /// Download ID.
        id: String,
    },
    /// Resume a paused download.
    Resume {
        /// Download ID.
        id: String,
    },
    /// Remove a download.
    Remove {
        /// Download ID.
        id: String,
    },
    /// List all downloads.
    List,
    /// Show download status.
    Status {
        /// Download ID.
        id: String,
    },
}

fn parse_priority(s: &str) -> Result<DownloadPriority, String> {
    match s.to_lowercase().as_str() {
        "low" => Ok(DownloadPriority::Low),
        "normal" => Ok(DownloadPriority::Normal),
        "high" => Ok(DownloadPriority::High),
        "critical" => Ok(DownloadPriority::Critical),
        other => Err(format!(
            "unknown priority '{other}', expected one of: low, normal, high, critical"
        )),
    }
}

fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let engine = DownloadEngine::builder().build()?;

    match cli.command {
        Commands::Dl(dl_cmd) => match dl_cmd {
            DlCommands::Add {
                url,
                output,
                priority,
            } => {
                let destination = output.unwrap_or_else(|| {
                    url.split('/').next_back().unwrap_or("download").to_string()
                });
                let priority = parse_priority(&priority).map_err(|e| eyre::eyre!("{e}"))?;
                let request = hpx_dl::DownloadRequest::builder(&url, &destination)
                    .priority(priority)
                    .build();
                let id = engine.add(request)?;
                println!("Added download {id}");
            }
            DlCommands::Pause { id } => {
                let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
                engine.pause(download_id)?;
                println!("Paused {download_id}");
            }
            DlCommands::Resume { id } => {
                let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
                engine.resume(download_id)?;
                println!("Resumed {download_id}");
            }
            DlCommands::Remove { id } => {
                let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
                engine.remove(download_id)?;
                println!("Removed {download_id}");
            }
            DlCommands::List => {
                let downloads = engine.list()?;
                if downloads.is_empty() {
                    println!("No downloads.");
                } else {
                    for status in &downloads {
                        println!(
                            "{}  {}  {}  {}/{} bytes  {:?}",
                            status.id,
                            status.state,
                            status.url,
                            status.bytes_downloaded,
                            status
                                .total_bytes
                                .map_or_else(|| "?".to_string(), |t| t.to_string()),
                            status.priority,
                        );
                    }
                }
            }
            DlCommands::Status { id } => {
                let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
                let status = engine.status(download_id)?;
                println!(
                    "ID:       {}\nURL:      {}\nState:    {}\nProgress: {}/{} bytes\nPriority: {:?}",
                    status.id,
                    status.url,
                    status.state,
                    status.bytes_downloaded,
                    status
                        .total_bytes
                        .map_or_else(|| "?".to_string(), |t| t.to_string()),
                    status.priority,
                );
            }
        },
    }

    Ok(())
}
