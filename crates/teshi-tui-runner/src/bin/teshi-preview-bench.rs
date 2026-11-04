//! Measures conversion of an already captured immutable SVG, without a TUI.

use anyhow::{Context, Result};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    anyhow::ensure!(
        args.len() == 2,
        "usage: teshi-preview-bench <source.svg> <output-directory>"
    );
    let source = std::path::Path::new(&args[0]);
    let output = std::path::Path::new(&args[1]);
    std::fs::create_dir_all(output).context("create benchmark output directory")?;
    for sample in 1..=3 {
        let timings = teshi_tui_runner::preview::render_svg(
            source,
            &output.join(format!("sample-{sample}.png")),
        )?;
        println!("{}", serde_json::json!({"sample":sample,"timings":timings}));
    }
    Ok(())
}
