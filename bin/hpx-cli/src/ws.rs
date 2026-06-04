use std::io::{self, BufRead, IsTerminal, Read, Write};

use eyre::{ContextCompat, Result, WrapErr};
use hpx::ws::message::Message;

use crate::{
    cli::Cli,
    output::{format_json_pretty, is_terminal, write_body},
};

pub(crate) async fn execute(cli: &Cli, url: &str) -> Result<()> {
    let mut builder = hpx::websocket(url);

    for (name, value) in &cli.parsed_headers() {
        builder = builder.header(name.as_str(), value.as_str());
    }

    if let Some(ref user) = cli.bearer {
        builder = builder.bearer_auth(user);
    } else if let Some(ref basic) = cli.basic {
        let (user, pass) = basic
            .split_once(':')
            .wrap_err("basic auth must be in USER:PASS format")?;
        builder = builder.basic_auth(user, Some(pass));
    }

    let resp = builder
        .send()
        .await
        .wrap_err("WebSocket handshake failed")?;

    let mut ws = resp
        .into_websocket()
        .await
        .wrap_err("Failed to upgrade to WebSocket")?;

    if !cli.silent {
        eprintln!("Connected to {url}");
    }

    // Send initial message from --data or --json
    if let Some(ref data) = cli.data {
        let msg = load_data_payload(data)?;
        ws.send(msg)
            .await
            .wrap_err("Failed to send initial message")?;
    } else if let Some(ref json_data) = cli.json {
        let msg = load_json_payload(json_data)?;
        ws.send(msg)
            .await
            .wrap_err("Failed to send initial message")?;
    }

    // Determine interactive vs non-interactive mode
    let stdin = io::stdin();
    let is_interactive = stdin.is_terminal() && cli.data.is_none() && cli.json.is_none();

    if is_interactive {
        run_interactive(&mut ws, cli).await?;
    } else {
        run_stdin_pump(&mut ws, cli).await?;
    }

    // Graceful close
    ws.close(hpx::ws::message::CloseCode::NORMAL, "")
        .await
        .wrap_err("Failed to close WebSocket")?;

    if !cli.silent {
        eprintln!("Connection closed");
    }

    Ok(())
}

async fn run_interactive(ws: &mut hpx::ws::WebSocket, cli: &Cli) -> Result<()> {
    if !cli.silent {
        eprintln!("Interactive mode. Type messages and press Enter. Ctrl+D to exit.");
    }

    let stdin = io::stdin();
    let mut reader = stdin.lock();

    loop {
        if !cli.silent {
            eprint!("> ");
            io::stderr().flush()?;
        }

        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
                if trimmed.is_empty() {
                    continue;
                }
                let msg = Message::text(trimmed.to_string());
                if let Err(e) = ws.send(msg).await {
                    eprintln!("Send error: {e}");
                    break;
                }
            }
            Err(e) => {
                eprintln!("Read error: {e}");
                break;
            }
        }

        // Check for pending messages (non-blocking drain attempt)
        drain_pending(ws, cli).await;
    }

    Ok(())
}

async fn run_stdin_pump(ws: &mut hpx::ws::WebSocket, cli: &Cli) -> Result<()> {
    let mut buf = Vec::new();
    io::stdin()
        .read_to_end(&mut buf)
        .wrap_err("Failed to read stdin")?;

    if !buf.is_empty() {
        let msg = Message::binary(buf);
        ws.send(msg).await.wrap_err("Failed to send stdin data")?;
    }

    // Now receive all responses until connection closes
    receive_all(ws, cli).await
}

async fn drain_pending(ws: &mut hpx::ws::WebSocket, cli: &Cli) {
    // Attempt to receive messages without blocking for too long
    // Use tokio::select with a short timeout
    loop {
        let msg = tokio::time::timeout(std::time::Duration::from_millis(10), ws.recv()).await;

        match msg {
            Ok(Some(Ok(message))) => print_message(message, cli),
            Ok(Some(Err(e))) => {
                eprintln!("Receive error: {e}");
                break;
            }
            Ok(None) => {
                eprintln!("Connection closed by server");
                break;
            }
            Err(_) => break, // timeout, no message pending
        }
    }
}

async fn receive_all(ws: &mut hpx::ws::WebSocket, cli: &Cli) -> Result<()> {
    loop {
        match ws.recv().await {
            Some(Ok(message)) => print_message(message, cli),
            Some(Err(e)) => {
                eprintln!("Receive error: {e}");
                break;
            }
            None => break,
        }
    }
    Ok(())
}

fn print_message(message: Message, cli: &Cli) {
    match message {
        Message::Text(text) => {
            let s = text.as_str();
            if cli.format == crate::cli::OutputFormat::Auto
                && is_json(s)
                && let Ok(pretty) = format_json_pretty(s.as_bytes())
            {
                println!("{pretty}");
                return;
            }
            println!("{s}");
        }
        Message::Binary(data) => {
            if let Some(ref path) = cli.output {
                if let Err(e) = write_body(&data, Some(path)) {
                    eprintln!("Failed to write binary to file: {e}");
                } else if !cli.silent {
                    eprintln!("Wrote {} bytes to {path}", data.len());
                }
            } else if is_terminal() {
                eprintln!(
                    "[binary message, {} bytes. Use --output to save to file.]",
                    data.len()
                );
            } else {
                let _ = io::stdout().write_all(&data);
            }
        }
        Message::Ping(data) => {
            if cli.verbose > 0 {
                eprintln!("[ping, {} bytes]", data.len());
            }
        }
        Message::Pong(data) => {
            if cli.verbose > 0 {
                eprintln!("[pong, {} bytes]", data.len());
            }
        }
        Message::Close(close) => {
            if let Some(frame) = close {
                eprintln!(
                    "[close, code={:?}, reason={}]",
                    frame.code,
                    frame.reason.as_str()
                );
            } else {
                eprintln!("[close]");
            }
        }
    }
}

fn is_json(s: &str) -> bool {
    let trimmed = s.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn load_data_payload(data: &str) -> Result<Message> {
    if let Some(path) = data.strip_prefix('@') {
        let bytes =
            std::fs::read(path).wrap_err_with(|| format!("Failed to read data file: {path}"))?;
        Ok(Message::binary(bytes))
    } else {
        Ok(Message::text(data.to_string()))
    }
}

fn load_json_payload(json: &str) -> Result<Message> {
    if let Some(path) = json.strip_prefix('@') {
        let bytes =
            std::fs::read(path).wrap_err_with(|| format!("Failed to read JSON file: {path}"))?;
        let _value: serde_json::Value =
            serde_json::from_slice(&bytes).wrap_err("Invalid JSON in file")?;
        Ok(Message::binary(bytes))
    } else {
        let _value: serde_json::Value =
            serde_json::from_str(json).wrap_err("Invalid JSON string")?;
        Ok(Message::text(json.to_string()))
    }
}
