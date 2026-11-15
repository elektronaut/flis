fn main() {
    std::process::exit(snags::cli::run(std::env::args_os().collect()));
}
