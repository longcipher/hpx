//! `hpx-dl` — High-performance download engine CLI.

#![allow(clippy::print_stdout)]
#![allow(missing_docs)]

use clap::Parser;
use hpx_dl::{
    DownloadEngine,
    cli::{Cli, Commands, parse_priority},
};

fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();

    let engine = DownloadEngine::builder().build();

    match cli.command {
        Commands::Add {
            url,
            output,
            priority,
        } => {
            let destination = output
                .unwrap_or_else(|| url.split('/').next_back().unwrap_or("download").to_string());
            let _priority = parse_priority(&priority).map_err(|e| eyre::eyre!("{e}"))?;
            let request = hpx_dl::DownloadRequest::builder(&url, &destination).build();
            let id = engine.add(request)?;
            println!("Added download {id}");
        }
        Commands::Pause { id } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            engine.pause(download_id)?;
            println!("Paused {download_id}");
        }
        Commands::Resume { id } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            engine.resume(download_id)?;
            println!("Resumed {download_id}");
        }
        Commands::Remove { id } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            engine.remove(download_id)?;
            println!("Removed {download_id}");
        }
        Commands::List => {
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
        Commands::Status { id } => {
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
    }

    Ok(())
}
