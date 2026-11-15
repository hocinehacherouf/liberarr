//! Liberarr CLI: parses the command, loads the config, wires the adapters and runs.
mod commands;
mod config;
mod container;
mod i18n;
mod models;
mod providers;
mod setup;
mod tui;

// Catalogs are embedded at compile time: no locale file is read at runtime.
rust_i18n::i18n!("locales", fallback = "en");

use clap::{Arg, ArgAction, Command, CommandFactory, FromArgMatches, Parser, Subcommand};
use rust_i18n::t;
use std::io::Write;
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::ExitCode;

// The help text is not here: `command()` reads it from the catalogs (`help.<command>.<arg>`).
#[derive(Parser)]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    Init {
        #[arg(long)]
        non_interactive: bool,
        #[arg(long, value_parser = ["plain", "tui"])]
        ui: Option<String>,
    },
    Link {
        #[arg(long)]
        force: bool,
    },
    Pull {
        #[arg(long)]
        no_process: bool,
    },
    Process {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    Rerun {
        id: i64,
        #[arg(long, value_parser = liberarr_core::jobs::STEPS)]
        step: Option<String>,
    },
    Serve {
        #[arg(long, default_value = "127.0.0.1")]
        host: IpAddr,
        #[arg(long, default_value_t = 8484)]
        port: u16,
    },
    Mcp,
    Status,
    Tui,
}

/// The clap command with every heading and help line in the current locale. Clap's own words
/// (default, possible values, the built-in help and version flags) are replaced by catalog text.
fn command() -> Command {
    localize(Cli::command(), "help")
}

fn localize(mut cmd: Command, key: &str) -> Command {
    let options = t!("help.options").into_owned();
    let arguments = t!("help.arguments").into_owned();
    let names: Vec<String> = cmd
        .get_subcommands()
        .map(|s| s.get_name().to_string())
        .collect();
    for name in names {
        cmd = cmd.mut_subcommand(&name, |s| localize(s, &format!("help.{name}")));
    }
    cmd = cmd
        .about(t!(format!("{key}.about")))
        .help_template(format!(
            "{{about-with-newline}}\n{} {{usage}}\n\n{{all-args}}",
            t!("help.usage")
        ))
        .subcommand_help_heading(t!("help.commands").into_owned())
        .subcommand_value_name(t!("help.command").into_owned())
        // Its text is clap's, in English only; `<command> --help` does the same job.
        .disable_help_subcommand(true)
        .disable_help_flag(true)
        .mut_args(|a| {
            let default: Vec<_> = a
                .get_default_values()
                .iter()
                .map(|v| v.to_string_lossy())
                .collect();
            let values: Vec<_> = a
                .get_possible_values()
                .into_iter()
                .map(|v| v.get_name().to_string())
                .collect();
            let help = t!(
                format!("{key}.{}", a.get_id()),
                default = default.join(", "),
                values = values.join(", ")
            );
            let heading = if a.is_positional() {
                &arguments
            } else {
                &options
            };
            // Clap's English "[default: …]" and "[possible values: …]" are in the catalog text.
            let takes_value = a.get_action().takes_values();
            a.help(help)
                .help_heading(heading.clone())
                .hide_default_value(takes_value)
                .hide_possible_values(takes_value)
        })
        .arg(flag("help", 'h', ArgAction::Help, &options));
    if cmd.get_version().is_some() {
        cmd =
            cmd.disable_version_flag(true)
                .arg(flag("version", 'V', ArgAction::Version, &options));
    }
    cmd
}

/// Clap's built-in `--help` / `--version`, with the catalog text (`help.help`, `help.version`).
fn flag(name: &'static str, short: char, action: ArgAction, heading: &str) -> Arg {
    Arg::new(name)
        .short(short)
        .long(name)
        .action(action)
        .help(t!(format!("help.{name}")))
        .help_heading(heading.to_string())
}

fn run(cmd: Cmd) -> commands::Outcome {
    if let Cmd::Init {
        non_interactive,
        ui,
    } = cmd
    {
        let ui = ui.or_else(|| std::env::var("LIBERARR_UI").ok());
        return commands::init::run(non_interactive, ui.as_deref() == Some("tui"));
    }
    if let Cmd::Tui = cmd {
        return tui::run();
    }
    let cfg = config::load(&config::home()?)?;
    let rt = tokio::runtime::Runtime::new()?;
    if let Cmd::Link { force } = cmd {
        return rt.block_on(commands::link::run(&cfg, force));
    }
    for w in container::warnings(&cfg) {
        eprintln!("{w}");
    }
    // Synchronous open (it may wait on a busy database), before any async work.
    let app = container::App::open(cfg)?;
    rt.block_on(async {
        match cmd {
            Cmd::Pull { no_process } => commands::pull::run(&app, no_process).await,
            Cmd::Process { files } => commands::process::run(&app, &files).await,
            Cmd::Rerun { id, step } => commands::rerun::run(&app, id, step.as_deref()).await,
            Cmd::Serve { host, port } => commands::serve::run(app, host, port).await,
            Cmd::Mcp => commands::mcp::run(&app).await,
            Cmd::Status => commands::status::run(&app).await,
            Cmd::Init { .. } | Cmd::Link { .. } | Cmd::Tui => unreachable!("handled above"),
        }
    })
}

fn main() -> ExitCode {
    // Once, before any message (the help included) is printed.
    let lang = config::home().map_or_else(|_| i18n::language(None), |h| i18n::ui_language(&h));
    rust_i18n::set_locale(lang.as_str());
    let cmd = match Cli::from_arg_matches(&command().get_matches()) {
        Ok(cli) => cli.cmd,
        Err(e) => e.exit(),
    };
    match run(cmd) {
        Ok(code) => code,
        Err(e) => {
            // Not `eprintln!`: it panics when the terminal has hung up.
            let message = rust_i18n::t!("error.prefix", message = e);
            let _ = writeln!(std::io::stderr(), "{message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    /// A command or option without a catalog entry would show its bare key (`help.…`) as help.
    /// The French catalog has the same keys (checked in `i18n`).
    #[test]
    fn every_command_and_option_has_help() {
        fn check(cmd: &clap::Command) {
            let about = cmd.get_about().map(ToString::to_string).unwrap_or_default();
            assert!(
                !about.is_empty() && !about.contains("help."),
                "{}: {about}",
                cmd.get_name()
            );
            for a in cmd.get_arguments() {
                let help = a.get_help().map(ToString::to_string).unwrap_or_default();
                assert!(
                    !help.is_empty() && !help.contains("help."),
                    "{}: {help}",
                    a.get_id()
                );
            }
            cmd.get_subcommands().for_each(check);
        }
        let mut cmd = super::command();
        // Clap's own consistency checks, on every subcommand (they run lazily otherwise).
        cmd.clone().debug_assert();
        cmd.build();
        check(&cmd);
    }
}
