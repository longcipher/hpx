use std::{
    io::{self, Write},
    time::Instant,
};

use hpx_dl::{DownloadEvent, DownloadState};

/// Renders download progress to stderr.
pub(crate) struct ProgressDisplay {
    start: Instant,
    last_update: Instant,
    is_terminal: bool,
}

impl ProgressDisplay {
    pub(crate) fn new(is_terminal: bool) -> Self {
        let now = Instant::now();
        Self {
            start: now,
            last_update: now,
            is_terminal,
        }
    }

    pub(crate) fn handle_event(&mut self, event: DownloadEvent) {
        match event {
            DownloadEvent::Started { id } => {
                if self.is_terminal {
                    eprintln!("[{id}] Download started");
                }
            }
            DownloadEvent::Progress {
                id,
                downloaded,
                total,
            } => {
                let now = Instant::now();
                // Throttle output to every 200ms
                if now.duration_since(self.last_update).as_millis() < 200 {
                    return;
                }
                self.last_update = now;

                if let Some(total) = total {
                    if total == 0 {
                        return;
                    }
                    let pct = (downloaded as f64 / total as f64 * 100.0) as u32;
                    let elapsed = now.duration_since(self.start).as_secs_f64();
                    let speed = if elapsed > 0.0 {
                        downloaded as f64 / elapsed
                    } else {
                        0.0
                    };
                    let eta = if speed > 0.0 {
                        (total - downloaded) as f64 / speed
                    } else {
                        0.0
                    };

                    if self.is_terminal {
                        let _ = write!(
                            io::stderr(),
                            "\r[{id}] {pct}% {}/{}  {}/s  ETA {:.0}s",
                            format_bytes(downloaded),
                            format_bytes(total),
                            format_bytes(speed as u64),
                            eta
                        );
                        let _ = io::stderr().flush();
                    } else {
                        eprintln!(
                            "[{id}] {pct}% {}/{}",
                            format_bytes(downloaded),
                            format_bytes(total)
                        );
                    }
                } else {
                    let elapsed = now.duration_since(self.start).as_secs_f64();
                    let speed = if elapsed > 0.0 {
                        downloaded as f64 / elapsed
                    } else {
                        0.0
                    };
                    if self.is_terminal {
                        let _ = write!(
                            io::stderr(),
                            "\r[{id}] {} downloaded  {}/s",
                            format_bytes(downloaded),
                            format_bytes(speed as u64)
                        );
                        let _ = io::stderr().flush();
                    } else {
                        eprintln!("[{id}] {} downloaded", format_bytes(downloaded));
                    }
                }
            }
            DownloadEvent::StateChanged { id, state } => match state {
                DownloadState::Completed => {
                    if self.is_terminal {
                        let _ = write!(io::stderr(), "\r");
                        let _ = io::stderr().flush();
                    }
                    eprintln!("[{id}] Download completed");
                }
                DownloadState::Failed => {
                    if self.is_terminal {
                        let _ = write!(io::stderr(), "\r");
                        let _ = io::stderr().flush();
                    }
                    eprintln!("[{id}] Download failed");
                }
                DownloadState::Connecting if self.is_terminal => {
                    eprintln!("[{id}] Connecting...");
                }
                _ => {}
            },
            DownloadEvent::Failed { id, error } => {
                if self.is_terminal {
                    let _ = write!(io::stderr(), "\r");
                    let _ = io::stderr().flush();
                }
                eprintln!("[{id}] Error: {error}");
            }
            _ => {}
        }
    }
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use hpx_dl::DownloadId;

    use super::*;

    #[test]
    fn format_bytes_zero() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn format_bytes_bytes() {
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn format_bytes_kilobytes() {
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn format_bytes_megabytes() {
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
    }

    #[test]
    fn format_bytes_gigabytes() {
        assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
    }

    #[test]
    fn progress_display_handles_completed_event() {
        let mut display = ProgressDisplay::new(false);
        let id = DownloadId::new();
        display.handle_event(DownloadEvent::StateChanged {
            id,
            state: DownloadState::Completed,
        });
        // Should not panic
    }
}
