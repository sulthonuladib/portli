//! portli - a small TUI for checking whether TCP ports on a host are open.
//!
//! Run with an optional host argument (`portli 192.168.1.1`, default
//! `127.0.0.1`). With an empty input the ten [`DEFAULT_PORTS`] are scanned.

mod app;
mod ports;
mod scan;
mod ui;

use std::io;
use std::ops::ControlFlow;
use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::time::Duration;

use crossterm::event::{self, Event};
use scan::PortProbe;

use crate::app::{Action, App};
use crate::scan::TcpProbe;

const DEFAULT_HOST: &str = "127.0.0.1";
const POLL_INTERVAL: Duration = Duration::from_millis(50);

enum Cli {
    Run { host: String },
    Help,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("portli: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> io::Result<()> {
    let host = match parse_args() {
        Ok(Cli::Help) => {
            print_usage();
            return Ok(());
        }
        Ok(Cli::Run { host }) => host,
        Err(err) => {
            eprintln!("portli: {err}\n");
            print_usage();
            return Ok(());
        }
    };

    let mut terminal = ratatui::init();
    let result = run_app(&mut terminal, host);
    // Always restore the terminal, also when `run_app` returned an error.
    ratatui::restore();
    result
}

fn run_app(terminal: &mut ratatui::DefaultTerminal, host: String) -> io::Result<()> {
    let mut app = App::new(host);
    let probe: Arc<dyn PortProbe> = Arc::new(TcpProbe);

    // Scan the defaults immediately so the app is useful on launch.
    let action = app.begin_scan();
    if execute(&mut app, action, &probe).is_break() {
        return Ok(());
    }

    loop {
        terminal.draw(|frame| ui::draw(frame, &app))?;

        // Poll with a timeout so results stream in even when no key is pressed.
        if event::poll(POLL_INTERVAL)?
            && let Event::Key(key) = event::read()?
        {
            let action = app.on_key(key);
            if execute(&mut app, action, &probe).is_break() {
                return Ok(());
            }
        }

        app.pump();
    }
}

/// Run the side effect requested by a key press.
fn execute(app: &mut App, action: Action, probe: &Arc<dyn PortProbe>) -> ControlFlow<()> {
    match action {
        Action::None => ControlFlow::Continue(()),
        Action::Quit => ControlFlow::Break(()),
        Action::Scan(params) => {
            let (tx, rx) = mpsc::channel();
            scan::spawn_scan(params, Arc::clone(probe), tx);
            app.attach(rx);
            ControlFlow::Continue(())
        }
    }
}

fn parse_args() -> Result<Cli, String> {
    let mut host: Option<String> = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => return Ok(Cli::Help),
            "-H" | "--host" => {
                let value = args.next().ok_or("`--host` needs a value")?;
                host = Some(value);
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown option `{other}`"));
            }
            other => host = Some(other.to_owned()),
        }
    }

    Ok(Cli::Run {
        host: host.unwrap_or_else(|| DEFAULT_HOST.to_owned()),
    })
}

fn print_usage() {
    println!(
        "portli - check TCP ports from a terminal UI

USAGE:
    portli [HOST]

ARGS:
    HOST    Host to scan (default: {DEFAULT_HOST})

OPTIONS:
    -H, --host <HOST>    Same as the positional HOST argument
    -h, --help           Print this help

Inside the TUI, leave the port field empty to scan the default ports:
{}
",
        ports::DEFAULT_PORTS
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
}
