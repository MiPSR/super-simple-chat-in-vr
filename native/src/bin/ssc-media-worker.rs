fn main() {
    if let Err(error) = super_simple_chat_native::media_worker_main() {
        eprintln!("Isolated media worker: {error}");
        std::process::exit(1);
    }
}
