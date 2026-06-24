//! `hpx` — CLI for high-performance HTTP client and download engine.

#![allow(clippy::print_stdout)]
#![allow(clippy::print_stderr)]
#![allow(missing_docs)]

mod browser;
mod cli;
mod http;
mod output;
mod progress;
mod proxy_test;
mod ws;

use clap::{CommandFactory, Parser};
use cli::Cli;

fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Apply timezone from env/flag
    if let Some(tz) = &cli.timezone {
        // SAFETY: called early in main, before any threads read TZ
        unsafe { std::env::set_var("TZ", tz) };
    }

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
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        return runtime.block_on(handle_dl_command(
            dl_cmd,
            cli.retry,
            cli.storage_path,
            cli.max_concurrent,
        ));
    }

    // Handle browser subcommands
    match cli.command {
        Some(cli::Commands::Fetch {
            url,
            dump,
            selector,
            wait,
            timeout,
            wait_until,
            eval,
            output,
            quiet,
        }) => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            return runtime.block_on(browser::handle_fetch(
                url,
                dump,
                selector,
                wait,
                timeout,
                wait_until,
                eval,
                output,
                quiet,
                cli.obey_robots,
                cli.allow_private_network,
                cli.v8_flags,
                cli.storage_dir,
            ));
        }
        Some(cli::Commands::Scrape {
            urls,
            eval,
            concurrency,
            format,
            timeout,
            quiet,
        }) => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            return runtime.block_on(browser::handle_scrape(
                urls,
                eval,
                concurrency,
                format,
                timeout,
                quiet,
                cli.obey_robots,
                cli.allow_private_network,
                cli.v8_flags,
                cli.storage_dir,
            ));
        }
        Some(cli::Commands::Serve {
            port,
            host,
            stealth,
            workers,
            allow_file_access,
            storage_dir,
            quiet,
        }) => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            return runtime.block_on(browser::handle_serve(
                port,
                host,
                stealth,
                workers,
                allow_file_access,
                storage_dir,
                quiet,
                cli.obey_robots,
                cli.allow_private_network,
                cli.v8_flags,
            ));
        }
        Some(cli::Commands::Dl(_)) => unreachable!(),
        Some(cli::Commands::ProxyTest { proxy }) => {
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            return runtime.block_on(proxy_test::run(&proxy));
        }
        None => {}
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

async fn handle_dl_command(
    cmd: cli::DlCommands,
    global_retry: u32,
    storage_path: Option<std::path::PathBuf>,
    max_concurrent: Option<usize>,
) -> eyre::Result<()> {
    let mut builder = hpx_dl::DownloadEngine::builder().retry_max_attempts(global_retry);
    if let Some(path) = storage_path {
        builder = builder.storage_path(path);
    }
    if let Some(max) = max_concurrent {
        builder = builder.max_concurrent(max);
    }
    let engine = builder.build()?;

    match cmd {
        cli::DlCommands::Add {
            url,
            output,
            priority,
            speed_limit,
            checksum,
            mirrors,
            max_connections,
            headers,
            proxy,
            retry: _,
        } => {
            let destination = output
                .unwrap_or_else(|| url.split('/').next_back().unwrap_or("download").to_string());
            let priority = parse_priority(&priority)?;
            let mut builder =
                hpx_dl::DownloadRequest::builder(&url, &destination).priority(priority);
            if let Some(limit_str) = speed_limit {
                let limit = cli::parse_speed_limit(&limit_str)?;
                builder = builder.speed_limit(limit);
            }
            if let Some(checksum_str) = checksum {
                let spec = cli::parse_checksum(&checksum_str)?;
                builder = builder.checksum(spec);
            }
            if !mirrors.is_empty() {
                builder = builder.mirrors(mirrors);
            }
            if let Some(max) = max_connections {
                builder = builder.max_connections(max);
            }
            for (name, value) in cli::parsed_dl_headers(&headers) {
                builder = builder.header(name, value);
            }
            if let Some(proxy_url) = proxy {
                let config = cli::parse_proxy_config(&proxy_url)?;
                builder = builder.proxy(config);
            }
            let request = builder.build();
            let id = engine.add(request)?;
            println!("Added download {id}");

            // Subscribe to events and display progress
            let mut rx = engine.subscribe();
            let is_terminal = crate::output::is_terminal();
            let mut display = crate::progress::ProgressDisplay::new(is_terminal);

            // Wait until download completes or fails
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let is_terminal_event = matches!(
                            event,
                            hpx_dl::DownloadEvent::StateChanged {
                                state: hpx_dl::DownloadState::Completed
                                    | hpx_dl::DownloadState::Failed,
                                ..
                            } | hpx_dl::DownloadEvent::Failed { .. }
                        );
                        display.handle_event(event);
                        if is_terminal_event {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                }
            }
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
        cli::DlCommands::List { format } => {
            let downloads = engine.list()?;
            if downloads.is_empty() {
                if matches!(format, cli::OutputFormat::Json) {
                    println!("[]");
                } else {
                    println!("No downloads.");
                }
            } else if matches!(format, cli::OutputFormat::Json) {
                let json = serde_json::to_string(&downloads)?;
                println!("{json}");
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
        cli::DlCommands::Status { id, format } => {
            let download_id = id.parse::<uuid::Uuid>().map(hpx_dl::DownloadId::from)?;
            let status = engine.status(download_id)?;
            if matches!(format, cli::OutputFormat::Json) {
                let json = serde_json::to_string(&status)?;
                println!("{json}");
            } else {
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
