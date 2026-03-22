//! Optional CLI interface for the download engine.
//!
//! Provides a command-line interface behind the `cli` feature flag using `clap`
//! for argument parsing. Commands: `add`, `pause`, `resume`, `remove`, `list`,
//! `status`.

use clap::{Parser, Subcommand};

/// High-performance download engine CLI.
#[derive(Debug, Parser)]
#[command(name = "hpx-dl", about = "High-performance download engine")]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,
}

/// Available CLI subcommands.
#[derive(Debug, Subcommand)]
pub enum Commands {
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

/// Parse priority string into a [`crate::DownloadPriority`].
///
/// # Errors
///
/// Returns an error string if the priority value is not recognized.
pub fn parse_priority(s: &str) -> Result<crate::DownloadPriority, String> {
    match s.to_lowercase().as_str() {
        "low" => Ok(crate::DownloadPriority::Low),
        "normal" => Ok(crate::DownloadPriority::Normal),
        "high" => Ok(crate::DownloadPriority::High),
        "critical" => Ok(crate::DownloadPriority::Critical),
        other => Err(format!(
            "unknown priority '{other}', expected one of: low, normal, high, critical"
        )),
    }
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn cli_parses_add_command() {
        let cli =
            Cli::try_parse_from(["hpx-dl", "add", "https://example.com/file.bin"]).expect("parse");
        match cli.command {
            Commands::Add {
                url,
                output,
                priority,
            } => {
                assert_eq!(url, "https://example.com/file.bin");
                assert!(output.is_none());
                assert_eq!(priority, "normal");
            }
            _ => panic!("expected Add command"),
        }
    }

    #[test]
    fn cli_parses_add_with_output() {
        let cli = Cli::try_parse_from([
            "hpx-dl",
            "add",
            "https://example.com/file.bin",
            "-o",
            "/tmp/out.bin",
        ])
        .expect("parse");
        match cli.command {
            Commands::Add {
                url,
                output,
                priority,
            } => {
                assert_eq!(url, "https://example.com/file.bin");
                assert_eq!(output.as_deref(), Some("/tmp/out.bin"));
                assert_eq!(priority, "normal");
            }
            _ => panic!("expected Add command"),
        }
    }

    #[test]
    fn cli_parses_add_with_priority() {
        let cli = Cli::try_parse_from([
            "hpx-dl",
            "add",
            "https://example.com/file.bin",
            "--priority",
            "high",
        ])
        .expect("parse");
        match cli.command {
            Commands::Add {
                url,
                output,
                priority,
            } => {
                assert_eq!(url, "https://example.com/file.bin");
                assert!(output.is_none());
                assert_eq!(priority, "high");
            }
            _ => panic!("expected Add command"),
        }
    }

    #[test]
    fn cli_parses_pause_command() {
        let cli = Cli::try_parse_from(["hpx-dl", "pause", "abc-123"]).expect("parse");
        match cli.command {
            Commands::Pause { id } => assert_eq!(id, "abc-123"),
            _ => panic!("expected Pause command"),
        }
    }

    #[test]
    fn cli_parses_resume_command() {
        let cli = Cli::try_parse_from(["hpx-dl", "resume", "abc-123"]).expect("parse");
        match cli.command {
            Commands::Resume { id } => assert_eq!(id, "abc-123"),
            _ => panic!("expected Resume command"),
        }
    }

    #[test]
    fn cli_parses_remove_command() {
        let cli = Cli::try_parse_from(["hpx-dl", "remove", "abc-123"]).expect("parse");
        match cli.command {
            Commands::Remove { id } => assert_eq!(id, "abc-123"),
            _ => panic!("expected Remove command"),
        }
    }

    #[test]
    fn cli_parses_list_command() {
        let cli = Cli::try_parse_from(["hpx-dl", "list"]).expect("parse");
        match cli.command {
            Commands::List => {}
            _ => panic!("expected List command"),
        }
    }

    #[test]
    fn cli_parses_status_command() {
        let cli = Cli::try_parse_from(["hpx-dl", "status", "abc-123"]).expect("parse");
        match cli.command {
            Commands::Status { id } => assert_eq!(id, "abc-123"),
            _ => panic!("expected Status command"),
        }
    }

    #[test]
    fn cli_fails_without_subcommand() {
        let result = Cli::try_parse_from(["hpx-dl"]);
        assert!(result.is_err());
    }

    #[test]
    fn cli_help_does_not_panic() {
        // clap returns Err for --help (it's a "display help" error, not a parse failure)
        let _result = Cli::try_parse_from(["hpx-dl", "--help"]);
    }

    #[test]
    fn parse_priority_valid_values() {
        assert_eq!(
            parse_priority("low").expect("valid"),
            crate::DownloadPriority::Low
        );
        assert_eq!(
            parse_priority("normal").expect("valid"),
            crate::DownloadPriority::Normal
        );
        assert_eq!(
            parse_priority("high").expect("valid"),
            crate::DownloadPriority::High
        );
        assert_eq!(
            parse_priority("critical").expect("valid"),
            crate::DownloadPriority::Critical
        );
    }

    #[test]
    fn parse_priority_case_insensitive() {
        assert_eq!(
            parse_priority("LOW").expect("valid"),
            crate::DownloadPriority::Low
        );
        assert_eq!(
            parse_priority("High").expect("valid"),
            crate::DownloadPriority::High
        );
        assert_eq!(
            parse_priority("CRITICAL").expect("valid"),
            crate::DownloadPriority::Critical
        );
    }

    #[test]
    fn parse_priority_invalid() {
        let result = parse_priority("urgent");
        assert!(result.is_err());
    }

    #[test]
    fn command_factory_builds_successfully() {
        Cli::command().debug_assert();
    }
}
