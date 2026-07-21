fn main() {
    // Ensure frontend dist exists — it is embedded by tauri::generate_context!() at compile time.
    // Unlike `tauri build`, plain `cargo build` does NOT run beforeBuildCommand (npm run build),
    // so we verify the dist is present and bail with a helpful message if not.
    let dist = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("frontend")
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
             \n     Before building with cargo build, run:\n\
             \n       npm --prefix apps/teshi-tauri/frontend run build\n\
             \n     Or build properly with:\n\
             \n       npm --prefix apps/teshi-tauri/frontend run build:desktop\n\
             \n     (which runs npm run build + tauri build --no-bundle)\n\
             \n     Or directly:\n\
             \n       npx --prefix apps/teshi-tauri/frontend tauri build --config apps/teshi-tauri/tauri.conf.json\n\
             \n     (which runs npm run build automatically via beforeBuildCommand)\n",
            index.display()
        );
    }
    // Tell cargo to re-run this script (and thus recompile teshi-tauri) whenever
    // any file in the frontend dist directory changes. Without this, cargo's
    // incremental compilation may skip recompilation when only the frontend changed,
    // leading to a binary with stale embedded UI and "localhost refused connection".
    if dist.is_dir() {
        println!("cargo:rerun-if-changed={}", dist.display());
    }
    tauri_build::build()
}
