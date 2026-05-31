use crate::config::Config;
use crate::error::Result;
use crate::metadata::index::Index;
use crate::output::Reporter;

pub async fn run(query: &str, _reporter: &dyn Reporter) -> Result<()> {
    let config = Config::from_env();
    let Some(index) = Index::load(&config)? else {
        println!("  Run `olma update --full` first for search");
        return Ok(());
    };

    let hits = index.substring(query);
    if !hits.is_empty() {
        for entry in hits.iter().take(40) {
            let desc = entry.desc.as_deref().unwrap_or("");
            println!("  {:<28} {desc}", entry.name);
        }
        if hits.len() > 40 {
            println!("  ... and {} more", hits.len() - 40);
        }
        return Ok(());
    }

    println!("  No formula matches \"{query}\".");
    let suggestions = index.did_you_mean(query, 3);
    if !suggestions.is_empty() {
        println!("\n  Did you mean:");
        for s in suggestions {
            println!("    {}", s.name);
        }
    }
    Ok(())
}
