//! `hpx` — CLI for high-performance HTTP client and download engine.

#![expect(clippy::print_stdout, reason = "CLI binary writes to stdout by design")]
#![expect(clippy::print_stderr, reason = "CLI binary writes to stderr by design")]

mod browser;
mod cli;
mod http;
mod output;
mod progress;
mod proxy_test;
mod ws;

use clap::{CommandFactory, Parser};
use cli::Cli;

fn build_runtime() -> eyre::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(Into::into)
}

fn main() -> eyre::Result<()> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Apply timezone from env/flag
    if let Some(tz) = &cli.timezone {
        // SAFETY: called early in main, before any threads read TZ
        unsafe { std::env::set_var("TZ", tz) };
    }

    // Warn about globally-declared flags that are parsed but not yet wired into
    // any code path, so users are not silently misled by --help.
    if cli.follow {
        tracing::warn!("--follow/-L is not yet implemented, ignoring");
    }
    if cli.redirects.is_some() {
        tracing::warn!("--redirects is not yet implemented, ignoring");
    }
    if cli.obey_robots {
        tracing::warn!("--obey-robots is not yet implemented, ignoring");
    }
    if cli.v8_flags.is_some() {
        tracing::warn!("--v8-flags is not yet implemented, ignoring");
    }
    if cli.storage_dir.is_some() {
        tracing::warn!("--storage-dir is not yet implemented, ignoring");
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
        let runtime = build_runtime()?;
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
            block,
        }) => {
            if !block.is_empty() {
                tracing::warn!("--block is not yet implemented, ignoring");
            }
            let runtime = build_runtime()?;
            let config = browser::FetchConfig {
                url,
                dump,
                selector,
                wait,
                timeout,
                wait_until,
                eval,
                output,
                quiet,
                allow_private_network: cli.allow_private_network,
            };
            return runtime.block_on(browser::handle_fetch(config));
        }
        Some(cli::Commands::Scrape {
            urls,
            eval,
            concurrency,
            format,
            timeout,
            quiet,
        }) => {
            let runtime = build_runtime()?;
            let config = browser::ScrapeConfig {
                urls,
                eval,
                concurrency,
                format,
                timeout,
                quiet,
                allow_private_network: cli.allow_private_network,
            };
            return runtime.block_on(browser::handle_scrape(config));
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
            if allow_file_access {
                tracing::warn!("--allow-file-access is not yet implemented, ignoring");
            }
            if storage_dir.is_some() {
                tracing::warn!("--storage-dir (serve) is not yet implemented, ignoring");
            }
            let runtime = build_runtime()?;
            let config = browser::ServeConfig {
                port,
                host,
                stealth,
                workers,
                quiet,
            };
            return runtime.block_on(browser::handle_serve(config));
        }
        Some(cli::Commands::Dl(_)) => {
            // Dl is handled by the `if let` above and returns early; reaching
            // here means a future refactor broke that invariant. Bail instead
            // of panicking so the safety net stays intact.
            eyre::bail!("internal error: Dl command reached unreachable handler");
        }
        Some(cli::Commands::ProxyTest { proxy }) => {
            let runtime = build_runtime()?;
            return runtime.block_on(proxy_test::run(&proxy));
        }
        None => {}
    }

    let runtime = build_runtime()?;

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
            tracing::error!("{e:#}");
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
            let request = builder.build()?;
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
