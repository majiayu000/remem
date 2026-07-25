use std::path::Path;

use anyhow::Result;

use super::super::archive_types::ExportArgs;
use super::markdown_archive::run_export_markdown;
use super::pack_export::run_export_pack;

pub(in crate::cli) fn run_export(args: ExportArgs, project: &str) -> Result<()> {
    if args.markdown {
        if args.pack.is_some() {
            anyhow::bail!("export accepts either --markdown or --pack, not both");
        }
        let Some(output) = args.output.as_deref() else {
            anyhow::bail!("markdown export requires --output <dir>");
        };
        return run_export_markdown(true, output, project, args.include_inactive, args.limit);
    }

    if args.output.is_some() {
        anyhow::bail!("pack export uses --pack <dir>; --output is only valid with --markdown");
    }
    if args.include_inactive {
        anyhow::bail!("pack export only includes active startup memories; --include-inactive is only valid with --markdown");
    }
    let pack_dir = args.pack.as_deref().unwrap_or(Path::new(".remem-pack"));
    run_export_pack(pack_dir, project, args.limit)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn export_rejects_markdown_and_pack_together() {
        let error = run_export(
            ExportArgs {
                markdown: true,
                output: Some(PathBuf::from("/tmp/remem-md")),
                pack: Some(PathBuf::from("/tmp/remem-pack")),
                project: None,
                include_inactive: false,
                limit: 100,
            },
            "/repo",
        )
        .expect_err("markdown and pack are mutually exclusive");

        assert!(
            error
                .to_string()
                .contains("either --markdown or --pack, not both"),
            "{error:?}"
        );
    }

    #[test]
    fn export_rejects_pack_with_markdown_only_flags() {
        let error = run_export(
            ExportArgs {
                markdown: false,
                output: Some(PathBuf::from("/tmp/remem-md")),
                pack: None,
                project: None,
                include_inactive: false,
                limit: 100,
            },
            "/repo",
        )
        .expect_err("pack export should not ignore --output");

        assert!(
            error
                .to_string()
                .contains("--output is only valid with --markdown"),
            "{error:?}"
        );
    }
}
