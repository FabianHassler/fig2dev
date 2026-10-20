//! fig2dev: fig2dev 3.2.9a's command line (the program is
//! `fig2dev::cli::run`).

use std::process::ExitCode;

fn main() -> ExitCode {
    // Arguments that aren't UTF-8 are read lossily rather than aborting.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let code = fig2dev::cli::run(
        args,
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &mut std::io::stderr().lock(),
    );
    ExitCode::from(code)
}
