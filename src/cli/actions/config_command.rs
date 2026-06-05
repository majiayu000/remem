use anyhow::Result;

use crate::cli::types::ConfigAction;

pub(in crate::cli) fn run_config(action: ConfigAction) -> Result<()> {
    match action {
        ConfigAction::Path => {
            println!("{}", crate::runtime_config::config_path().display());
        }
        ConfigAction::Show => {
            print!("{}", crate::runtime_config::show_config_text()?);
        }
        ConfigAction::Init => {
            let path = crate::runtime_config::init_config()?;
            println!("config -> {}", path.display());
        }
        ConfigAction::Set { key, value } => {
            let path = crate::runtime_config::set_config_value(&key, &value)?;
            println!("config -> {}", path.display());
        }
    }
    Ok(())
}
