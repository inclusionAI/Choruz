//! Command-line argument parsing for choruz-replay.
//!
//! Uses a simple hand-rolled parser (no clap dependency) to keep the binary
//! small.  The parser supports the flag combinations described in the plan
//! Section 10.3.

/// Parsed CLI arguments.
#[derive(Debug)]
pub struct Args {
    pub mode: ReplayMode,
    pub json: bool,
}

/// What to replay.
#[derive(Debug)]
pub enum ReplayMode {
    Conversation {
        conversation_id: String,
        from_seq: i64,
        to_seq: Option<i64>,
    },
    TurnId {
        turn_id: String,
    },
    CommandId {
        command_id: String,
    },
    DeadLetters {
        since_hours: i64,
    },
}

/// Parse command-line arguments.
///
/// Exits with a help message if the arguments are invalid.
pub fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().collect();

    let mut conversation_id: Option<String> = None;
    let mut from_seq: i64 = 0;
    let mut to_seq: Option<i64> = None;
    let mut turn_id: Option<String> = None;
    let mut command_id: Option<String> = None;
    let mut dead_letters = false;
    let mut since_hours: i64 = 24;
    let mut json = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--conversation" | "-c" => {
                i += 1;
                conversation_id = Some(require_value(&args, i, "--conversation"));
            }
            "--from-seq" => {
                i += 1;
                from_seq = require_value(&args, i, "--from-seq")
                    .parse()
                    .unwrap_or_else(|_| {
                        eprintln!("error: --from-seq must be a number");
                        std::process::exit(1);
                    });
            }
            "--to-seq" => {
                i += 1;
                to_seq = Some(
                    require_value(&args, i, "--to-seq")
                        .parse()
                        .unwrap_or_else(|_| {
                            eprintln!("error: --to-seq must be a number");
                            std::process::exit(1);
                        }),
                );
            }
            "--turn-id" | "-t" => {
                i += 1;
                turn_id = Some(require_value(&args, i, "--turn-id"));
            }
            "--command-id" => {
                i += 1;
                command_id = Some(require_value(&args, i, "--command-id"));
            }
            "--dead-letters" => {
                dead_letters = true;
            }
            "--since" => {
                i += 1;
                since_hours = parse_duration(&require_value(&args, i, "--since"));
            }
            "--json" => {
                json = true;
            }
            "--help" | "-h" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("error: unknown argument: {other}");
                print_help();
                std::process::exit(1);
            }
        }
        i += 1;
    }

    let mode = if dead_letters {
        ReplayMode::DeadLetters { since_hours }
    } else if let Some(tid) = turn_id {
        ReplayMode::TurnId { turn_id: tid }
    } else if let Some(cid) = command_id {
        ReplayMode::CommandId { command_id: cid }
    } else if let Some(conv) = conversation_id {
        ReplayMode::Conversation {
            conversation_id: conv,
            from_seq,
            to_seq,
        }
    } else {
        eprintln!("error: must specify --conversation, --turn-id, --command-id, or --dead-letters");
        print_help();
        std::process::exit(1);
    };

    Args { mode, json }
}

fn require_value(args: &[String], i: usize, flag: &str) -> String {
    if i >= args.len() {
        eprintln!("error: {flag} requires a value");
        std::process::exit(1);
    }
    args[i].clone()
}

/// Parse a duration string like "24h", "48h", "7d".
fn parse_duration(s: &str) -> i64 {
    let s = s.trim();
    if let Some(hours) = s.strip_suffix('h') {
        hours.parse().unwrap_or(24)
    } else if let Some(days) = s.strip_suffix('d') {
        days.parse::<i64>().unwrap_or(1) * 24
    } else {
        s.parse().unwrap_or(24)
    }
}

fn print_help() {
    eprintln!(
        "choruz-replay: replay conversation events for debugging

USAGE:
    choruz-replay --conversation <id> [--from-seq N] [--to-seq N] [--json]
    choruz-replay --turn-id <id> [--json]
    choruz-replay --command-id <id> [--json]
    choruz-replay --dead-letters [--since 24h] [--json]

OPTIONS:
    -c, --conversation <id>    Replay events for a conversation
    --from-seq <N>             Start from sequence N (default: 0)
    --to-seq <N>               Stop at sequence N (default: latest)
    -t, --turn-id <id>         Replay events for a specific turn
    --command-id <id>          Show command details and attempts
    --dead-letters             List unresolved dead letters
    --since <duration>         Time window for dead letters (e.g. 24h, 7d)
    --json                     Output as JSON (one object per line)
    -h, --help                 Show this help

ENVIRONMENT:
    CHORUZ_DATABASE_URL        libpq connection string
    RUST_LOG                   Tracing filter (default: warn)"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_duration_hours() {
        assert_eq!(parse_duration("24h"), 24);
        assert_eq!(parse_duration("48h"), 48);
    }

    #[test]
    fn parse_duration_days() {
        assert_eq!(parse_duration("7d"), 168);
        assert_eq!(parse_duration("1d"), 24);
    }

    #[test]
    fn parse_duration_plain_number() {
        assert_eq!(parse_duration("12"), 12);
    }

    #[test]
    fn parse_duration_invalid() {
        assert_eq!(parse_duration("foo"), 24); // default
    }
}
