use std::{
    io::{self, IsTerminal, Write},
    time::Instant,
};

use crate::cli::ColorMode;

pub(crate) const fn use_color(mode: ColorMode, is_terminal: bool) -> bool {
    match mode {
        ColorMode::On => true,
        ColorMode::Off => false,
        ColorMode::Auto => is_terminal,
    }
}

pub(crate) fn print_status_line(status: u16, version: &str, color: bool) {
    let mut stderr = io::stderr().lock();
    let _ = if color {
        let code = status_color_code(status);
        writeln!(stderr, "\x1b[{code}m{status}\x1b[0m {version}")
    } else {
        writeln!(stderr, "{status} {version}")
    };
}

pub(crate) fn print_headers(headers: &[(String, String)], verbose: bool, color: bool) {
    let mut stderr = io::stderr().lock();
    for (name, value) in headers {
        let _ = if color && verbose {
            writeln!(stderr, "\x1b[1;36m{name}\x1b[0m: {value}")
        } else {
            writeln!(stderr, "{name}: {value}")
        };
    }
}

pub(crate) fn print_request_line(method: &str, url: &str, color: bool) {
    let mut stderr = io::stderr().lock();
    let _ = if color {
        writeln!(stderr, "\x1b[1;33m{method}\x1b[0m \x1b[1;36m{url}\x1b[0m")
    } else {
        writeln!(stderr, "{method} {url}")
    };
}

pub(crate) fn write_body(body: &[u8], output: Option<&str>) -> eyre::Result<()> {
    if let Some(path) = output {
        std::fs::write(path, body)?;
    } else {
        let mut stdout = io::stdout().lock();
        stdout.write_all(body)?;
    }
    Ok(())
}

pub(crate) fn format_json_pretty(body: &[u8]) -> eyre::Result<String> {
    let value: serde_json::Value = serde_json::from_slice(body)?;
    Ok(serde_json::to_string_pretty(&value)?)
}

pub(crate) fn is_terminal() -> bool {
    std::io::stdout().is_terminal()
}

pub(crate) fn looks_like_json_str(s: &str) -> bool {
    let trimmed = s.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

#[expect(dead_code)]
pub(crate) fn bytes_appear_printable(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    let preview = if bytes.len() > 1024 {
        &bytes[..1024]
    } else {
        bytes
    };
    if preview.contains(&0) {
        return false;
    }
    let printable = preview
        .iter()
        .filter(|&&b| b >= 0x20 || b == b'\n' || b == b'\r' || b == b'\t')
        .count();
    printable as f64 / preview.len() as f64 >= 0.9
}

const fn status_color_code(status: u16) -> u8 {
    match status {
        200..=299 => 32, // green
        300..=399 => 33, // yellow
        400..=599 => 31, // red
        _ => 0,
    }
}

pub(crate) struct TimingWaterfall {
    request_start: Instant,
    request_done: Option<Instant>,
    body_done: Option<Instant>,
}

impl TimingWaterfall {
    pub(crate) fn new() -> Self {
        Self {
            request_start: Instant::now(),
            request_done: None,
            body_done: None,
        }
    }

    pub(crate) fn mark_request_done(&mut self) {
        self.request_done = Some(Instant::now());
    }

    pub(crate) fn mark_body_done(&mut self) {
        self.body_done = Some(Instant::now());
    }

    pub(crate) fn print(&self, color: bool) {
        let mut stderr = io::stderr().lock();
        let _ = writeln!(stderr);
        let _ = writeln!(stderr, "Timing waterfall:");

        let label_color = if color { "\x1b[1;36m" } else { "" };
        let reset = if color { "\x1b[0m" } else { "" };
        let dur_color = if color { "\x1b[1;33m" } else { "" };

        let connection = self
            .request_done
            .map(|t| t.duration_since(self.request_start));
        let ttfb = self.request_done.and_then(|req_done| {
            self.body_done
                .map(|body_done| body_done.duration_since(req_done))
        });
        let total = self.body_done.map(|t| t.duration_since(self.request_start));

        if let Some(dur) = connection {
            let ms = dur.as_secs_f64() * 1000.0;
            let _ = writeln!(
                stderr,
                "  {label_color}{:<12}{reset} {dur_color}{ms:>8.2} ms{reset}",
                "Connection"
            );
        }

        if let Some(dur) = ttfb {
            let ms = dur.as_secs_f64() * 1000.0;
            let _ = writeln!(
                stderr,
                "  {label_color}{:<12}{reset} {dur_color}{ms:>8.2} ms{reset}",
                "TTFB"
            );
        }

        if let (Some(req_done), Some(body_done)) = (self.request_done, self.body_done) {
            let transfer = body_done.duration_since(req_done);
            let ms = transfer.as_secs_f64() * 1000.0;
            let _ = writeln!(
                stderr,
                "  {label_color}{:<12}{reset} {dur_color}{ms:>8.2} ms{reset}",
                "Transfer"
            );
        }

        if let Some(dur) = total {
            let ms = dur.as_secs_f64() * 1000.0;
            let _ = writeln!(
                stderr,
                "  {label_color}{:<12}{reset} {dur_color}{ms:>8.2} ms{reset}",
                "Total"
            );
        }

        let _ = writeln!(stderr);
    }
}

pub(crate) fn print_redirect_history(extensions: &http::Extensions, verbose: bool, color: bool) {
    use hpx::redirect::History;

    if !verbose {
        return;
    }

    if let Some(history) = extensions.get::<History>() {
        let mut stderr = io::stderr().lock();
        let label = if color { "\x1b[1;36m" } else { "" };
        let reset = if color { "\x1b[0m" } else { "" };
        let code_color = if color { "\x1b[1;33m" } else { "" };

        for entry in history {
            let _ = writeln!(
                stderr,
                "{label}Redirect:{reset} {code_color}{}{reset} {} -> {}",
                entry.status.as_u16(),
                entry.previous,
                entry.uri
            );
        }
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn copy_to_clipboard(text: &str) -> eyre::Result<()> {
    use std::process::Command;
    let mut child = Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| eyre::eyre!("failed to run pbcopy: {e}"))?;
    use std::io::Write as _;
    if let Some(ref mut stdin) = child.stdin {
        stdin.write_all(text.as_bytes())?;
    }
    child.wait()?;
    Ok(())
}

#[cfg(not(target_os = "macos"))]
pub(crate) fn copy_to_clipboard(text: &str) -> eyre::Result<()> {
    use std::process::Command;

    // Try xclip first, then xsel
    if let Ok(mut child) = Command::new("xclip")
        .args(["-selection", "clipboard"])
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        use std::io::Write as _;
        if let Some(ref mut stdin) = child.stdin {
            stdin.write_all(text.as_bytes())?;
        }
        child.wait()?;
        return Ok(());
    }

    if let Ok(mut child) = Command::new("xsel")
        .args(["--clipboard", "--input"])
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        use std::io::Write as _;
        if let Some(ref mut stdin) = child.stdin {
            stdin.write_all(text.as_bytes())?;
        }
        child.wait()?;
        return Ok(());
    }

    Err(eyre::eyre!(
        "clipboard requires xclip or xsel (install via package manager)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_waterfall_new_has_no_phases() {
        let waterfall = TimingWaterfall::new();
        assert!(waterfall.request_done.is_none());
        assert!(waterfall.body_done.is_none());
    }

    #[test]
    fn timing_waterfall_mark_request_done() {
        let mut waterfall = TimingWaterfall::new();
        waterfall.mark_request_done();
        assert!(waterfall.request_done.is_some());
    }

    #[test]
    fn timing_waterfall_mark_body_done() {
        let mut waterfall = TimingWaterfall::new();
        waterfall.mark_request_done();
        waterfall.mark_body_done();
        assert!(waterfall.body_done.is_some());
    }

    #[test]
    fn timing_waterfall_print_does_not_panic() {
        let mut waterfall = TimingWaterfall::new();
        waterfall.mark_request_done();
        waterfall.mark_body_done();
        waterfall.print(false);
        waterfall.print(true);
    }

    #[test]
    fn timing_waterfall_print_no_phases_does_not_panic() {
        let waterfall = TimingWaterfall::new();
        waterfall.print(false);
    }
}
