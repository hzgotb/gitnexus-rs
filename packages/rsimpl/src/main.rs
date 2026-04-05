mod cli;
mod commands;
mod git;
mod ingestion;
mod storage;

fn main() {
    if let Err(err) = cli::run() {
        eprintln!("{err:#}");
        std::process::exit(1);
    }
}
