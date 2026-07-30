//! The noctalia binary. CLI subcommands land here across Phase 4 (task 4.2); the full shell
//! entry point (event loop, daemonization, ...) arrives in task 16.5. See MIGRATION_PLAN.md.
//!
//! Top-level dispatch only — each subcommand's own argument grammar is a 1:1 port of its C++
//! `runCli`, not re-expressed as clap derive attributes (the C++ grammars, e.g. theme's `-r
//! <in:out>` repeatable flag, don't map cleanly onto clap's declarative model, and re-deriving
//! them risks silently drifting from the C++). `disable_help_flag` on every subcommand stops
//! clap from intercepting `--help`/`-h` itself — `msg`'s `--help` in particular is a real IPC
//! command forwarded to the running instance (`IpcService::execute` special-cases it
//! server-side), not something this binary should ever answer locally.

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "noctalia", disable_help_subcommand = true)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Forward a command to the running noctalia instance over its IPC socket.
    #[command(disable_help_flag = true)]
    Msg {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Config utilities (validate, export, ...).
    #[command(disable_help_flag = true)]
    Config {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Generate a color palette/theme from an image.
    #[command(disable_help_flag = true)]
    Theme {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
}

fn main() {
    let cli = Cli::parse();

    let exit_code = match cli.command {
        Some(Command::Msg { args }) => noctalia_ipc::cli::run_cli(&args),
        Some(Command::Config { args }) => noctalia_config::cli::run_cli(&args),
        Some(Command::Theme { args }) => noctalia_theme::cli::run_cli(&args),
        None => {
            eprintln!(
                "noctalia-rs: migration skeleton -- shell startup arrives in task 16.5 (see MIGRATION_PLAN.md)"
            );
            1
        }
    };

    std::process::exit(exit_code);
}
