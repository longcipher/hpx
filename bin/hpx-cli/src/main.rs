//! `hpx` — CLI for high-performance HTTP client and download engine.

#![allow(clippy::print_stdout)]
#![allow(clippy::print_stderr)]
#![allow(missing_docs)]

mod cli;
mod http;
mod output;
mod ws;

use clap::{CommandFactory, Parser};
use cli::Cli;

fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    if cli.version {
        println!("hpx {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    if cli.help {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        return Ok(());
    }

    // Handle dl subcommand
    if let Some(cli::Commands::Dl(dl_cmd)) = cli.command {
        return handle_dl_command(dl_cmd);
    }

    // Validate URL is provided for HTTP/WS
    if cli.url.is_none() {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        eprintln!("\nError: URL is required");
        std::process::exit(1);
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    let Some(url) = cli.url.as_deref() else {
        let mut cmd = Cli::command();
        cmd.print_help()?;
        eprintln!("\nError: URL is required");
        std::process::exit(1);
    };

    let result = if cli.is_websocket_url() {
        runtime.block_on(ws::execute(&cli, url))
    } else {
        runtime.block_on(http::execute(&cli))
    };

    match result {
        Ok(()) => Ok(()),
        Err(e) => {
            eprintln!("error: {e:#}");
            std::process::exit(1);
        }
    }
}

fn handle_dl_command(cmd: cli::DlCommands) -> eyre::Result<()> {
    let engine = hpx_dl::DownloadEngine::builder().build()?;

    match cmd {
        cli::DlCommands::Add {
            url,
            output,
            priority,
        } => {
            let destination = output
                .unwrap_or_else(|| url.split('/').next_back().unwrap_or("download").to_string());
            let priority = parse_priority(&priority)?;
            let request = hpx_dl::DownloadRequest::builder(&url, &destination)
                .priority(priority)
                .build();
            let id = engine.add(request)?;
            println!("Added download {id}");
        }
        cli::DlCommands::Pause { id } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            engine.pause(download_id)?;
            println!("Paused {download_id}");
        }
        cli::DlCommands::Resume { id } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            engine.resume(download_id)?;
            println!("Resumed {download_id}");
        }
        cli::DlCommands::Remove { id } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            engine.remove(download_id)?;
            println!("Removed {download_id}");
        }
        cli::DlCommands::List => {
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
        cli::DlCommands::Status { id } => {
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

fn parse_priority(s: &str) -> eyre::Result<hpx_dl::DownloadPriority> {
    match s.to_lowercase().as_str() {
        "low" => Ok(hpx_dl::DownloadPriority::Low),
        "normal" => Ok(hpx_dl::DownloadPriority::Normal),
        "high" => Ok(hpx_dl::DownloadPriority::High),
        "critical" => Ok(hpx_dl::DownloadPriority::Critical),
        other => Err(eyre::eyre!(
            "unknown priority '{other}', expected one of: low, normal, high, critical"
        )),
    }
}
