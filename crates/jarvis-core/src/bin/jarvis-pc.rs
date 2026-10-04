// The MCP adapter only forwards requests; it never initializes a second voice core.
fn main() {
    if let Err(e) = jarvis_core::config::init_dirs().and_then(|_| {
        jarvis_core::agent::mcp::run(std::io::stdin().lock(), std::io::stdout().lock())
    }) {
        eprintln!("jarvis-pc: {}", e);
        std::process::exit(1);
    }
}
