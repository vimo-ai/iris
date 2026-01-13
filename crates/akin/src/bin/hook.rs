//! akin-hook - Claude Code PostToolUse hook 入口

#[tokio::main]
async fn main() {
    if let Err(e) = akin::run_hook().await {
        eprintln!("Hook error: {}", e);
        std::process::exit(1);
    }
}
