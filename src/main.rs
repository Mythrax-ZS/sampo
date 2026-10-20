//! `sampo`: an IRC server and an IRC client, in one binary.
//!
//! ```text
//! sampo serve                   run a server on port 6667
//! sampo connect [host[:port]]   connect to one
//! ```

mod cli;

use std::fmt::Display;
use std::process::ExitCode;
use std::time::Duration;

use sampo::{banner, client, server};

fn main() -> ExitCode {
    let command = match cli::parse(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("sampo: {error}");
            eprintln!("Try 'sampo --help'.");
            return ExitCode::from(2);
        }
    };
    match command {
        cli::Command::Help => {
            for line in banner::LOGO {
                println!("{line}");
            }
            println!("\n{}", cli::usage());
            ExitCode::SUCCESS
        }
        cli::Command::Version => {
            println!("sampo {}", banner::VERSION);
            ExitCode::SUCCESS
        }
        cli::Command::Serve(config) => run(serve(config)),
        cli::Command::Connect(options) => run(client::run(options)),
    }
}

/// Runs the program on a single thread, as IRC programs always ran: one
/// thread, one loop, every socket in it.
fn run<E: Display>(program: impl Future<Output = Result<(), E>>) -> ExitCode {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("sampo: cannot start: {error}");
            return ExitCode::FAILURE;
        }
    };
    let result = runtime.block_on(program);
    // A name lookup still under way must not hold up the exit.
    runtime.shutdown_timeout(Duration::from_millis(250));
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sampo: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn serve(config: server::Config) -> Result<(), server::Error> {
    for line in banner::LOGO {
        println!("{line}");
    }
    println!();
    let server = server::Server::bind(config).await?;
    server.run(server::shutdown_signal()).await;
    Ok(())
}
