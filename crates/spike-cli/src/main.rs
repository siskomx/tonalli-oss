fn main() {
    if let Err(error) = spike_cli::run(std::env::args().skip(1)) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
