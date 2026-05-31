use crate::config::Config;
use crate::error::{OlmaError, Result};
use crate::metadata::client::{FetchPolicy, FormulaeClient};
use crate::output::{default_reporter, Reporter};
use crate::pipeline::Pipeline;
use crate::platform::current_bottle_tag;
use crate::relocator::macho;
use crate::resolver::{InstallPlan, Resolver};
use std::sync::Arc;

pub async fn run(
    names: &[String],
    policy: FetchPolicy,
    yes: bool,
    dry_run: bool,
    reporter: &dyn Reporter,
) -> Result<()> {
    if names.is_empty() {
        return Err(OlmaError::Other("no packages specified".into()));
    }
    macho::check_clt_available()?;

    let config = Arc::new(Config::from_env());
    config.ensure_layout()?;

    reporter.status("Resolving dependencies");
    let client = FormulaeClient::new(&config)?;
    let resolver = Resolver::new(&client, policy);
    let plan = resolver.resolve(names).await?;

    let tag = current_bottle_tag().await?;
    print_plan(&plan, reporter);

    if dry_run {
        reporter.success("dry run — nothing installed");
        return Ok(());
    }
    if !confirm(yes)? {
        return Err(OlmaError::Other("aborted".into()));
    }

    let pipeline_reporter: Arc<dyn Reporter> = Arc::from(default_reporter());
    let pipeline = Pipeline::new(config, tag, pipeline_reporter);
    pipeline.run(plan).await?;
    Ok(())
}

fn print_plan(plan: &InstallPlan, reporter: &dyn Reporter) {
    let n = plan.ordered.len();
    reporter.status(&format!("Will install {n} packages"));
    for f in &plan.ordered {
        reporter.status(&format!("  {} {}", f.name, f.version()));
    }
}

fn confirm(yes: bool) -> Result<bool> {
    use std::io::IsTerminal;
    if yes {
        return Ok(true);
    }
    if !std::io::stdin().is_terminal() {
        return Err(OlmaError::Other("non-interactive: use -y".into()));
    }
    eprint!("Proceed? [Y/n] ");
    use std::io::Write;
    let _ = std::io::stderr().flush();
    let mut input = String::new();
    std::io::stdin().read_line(&mut input).map_err(OlmaError::Io)?;
    let trimmed = input.trim().to_lowercase();
    Ok(trimmed.is_empty() || trimmed == "y" || trimmed == "yes")
}
