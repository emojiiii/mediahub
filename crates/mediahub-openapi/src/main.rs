use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let check = env::args().skip(1).any(|argument| argument == "--check");
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../openapi/openapi.json");
    let generated = mediahub_openapi::to_pretty_json()?;

    if check {
        let current = std::fs::read_to_string(&output)
            .with_context(|| format!("failed to read {}", output.display()))?;
        if current != generated {
            bail!(
                "{} is stale; run `cargo run -p mediahub-openapi`",
                output.display()
            );
        }
        println!("OpenAPI contract is up to date: {}", output.display());
    } else {
        std::fs::write(&output, generated)
            .with_context(|| format!("failed to write {}", output.display()))?;
        println!("Generated {}", output.display());
    }

    Ok(())
}
