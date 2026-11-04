use std::io::BufRead;

fn main() {
    let result = (|| -> anyhow::Result<usize> {
        let line = std::io::stdin()
            .lock()
            .lines()
            .next()
            .transpose()?
            .ok_or_else(|| anyhow::anyhow!("missing NDJSON run request"))?;
        let request = serde_json::from_str(&line)?;
        teshi_tui_runner::scenario::run(request, &mut std::io::stdout().lock())
    })();
    match result {
        Ok(0) => {}
        Ok(_) => std::process::exit(1),
        Err(error) => {
            eprintln!("TUI runner error: {error:#}");
            std::process::exit(2);
        }
    }
}
