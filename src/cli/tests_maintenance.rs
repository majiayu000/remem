use super::types::{Cli, Commands};
use clap::Parser;

#[test]
fn cli_parses_top_level_cleanup_preview() {
    let cleanup = Cli::parse_from(["remem", "cleanup", "--dry-run", "--json"]);
    match cleanup.command {
        Commands::Cleanup { dry_run, json } => {
            assert!(dry_run);
            assert!(json);
        }
        _ => panic!("expected cleanup command"),
    }
}

#[test]
fn cli_parses_encrypt_rekey_raw() {
    let cli = Cli::parse_from(["remem", "encrypt", "--rekey-raw"]);
    match cli.command {
        Commands::Encrypt { rekey_raw } => assert!(rekey_raw),
        _ => panic!("expected encrypt command"),
    }
}

#[test]
fn cli_parses_backfill_embeddings_limit() {
    let cli = Cli::parse_from(["remem", "backfill-embeddings", "--limit", "250"]);
    match cli.command {
        Commands::BackfillEmbeddings { limit, batch_size } => {
            assert_eq!(limit, 250);
            assert_eq!(batch_size, 1000);
        }
        _ => panic!("expected backfill-embeddings command"),
    }
}

#[test]
fn cli_parses_reindex_embeddings_alias() {
    let cli = Cli::parse_from(["remem", "reindex-embeddings", "--limit", "50"]);
    match cli.command {
        Commands::BackfillEmbeddings { limit, batch_size } => {
            assert_eq!(limit, 50);
            assert_eq!(batch_size, 1000);
        }
        _ => panic!("expected reindex-embeddings alias"),
    }
}

#[test]
fn cli_parses_reindex_embeddings_batch_size() {
    let cli = Cli::parse_from([
        "remem",
        "reindex-embeddings",
        "--limit",
        "100000",
        "--batch-size",
        "5000",
    ]);
    match cli.command {
        Commands::BackfillEmbeddings { limit, batch_size } => {
            assert_eq!(limit, 100000);
            assert_eq!(batch_size, 5000);
        }
        _ => panic!("expected reindex-embeddings alias"),
    }
}
