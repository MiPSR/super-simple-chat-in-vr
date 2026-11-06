#[path = "support/world.rs"]
mod world;

fn main() -> anyhow::Result<()> {
    let destination = std::env::args_os()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("Usage: cargo run --example make-example -- OUTPUT.zip"))?;
    // Do not silently overwrite an author's existing package.
    let file = std::fs::File::create_new(destination)?;
    if std::env::args().any(|arg| arg == "--avatar") {
        world::write_avatar(file)
    } else if std::env::args().any(|arg| arg == "--tanks") {
        world::write_tanks(file)
    } else {
        world::write(file)
    }
}
