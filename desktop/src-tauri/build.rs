fn main() {
    // Ensure frontend dist exists — it is embedded by tauri::generate_context!() at compile time.
    // Unlike `tauri build`, plain `cargo build` does NOT run beforeBuildCommand (npm run build),
    // so we verify the dist is present and bail with a helpful message if not.
    let dist = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("..")
        .join("dist")
        .canonicalize()
        .unwrap_or_default();
    let index = dist.join("index.html");
    if !index.is_file()
        || std::fs::metadata(&index)
            .map(|m| m.len() == 0)
            .unwrap_or(true)
    {
        panic!(
            "\n\n  ❌ Frontend dist not found at {}\n\
             \n     Before building teshi-desktop with cargo build, run:\n\
             \n       cd desktop && npm run build\n\
             \n     Or use:  npx tauri build\n\
             \n     (which runs npm run build automatically via beforeBuildCommand)\n",
            index.display()
        );
    }
    tauri_build::build()
}
