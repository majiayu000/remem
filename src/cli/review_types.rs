use clap::Subcommand;

#[derive(Subcommand)]
pub(in crate::cli) enum ReviewAction {
    List {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 'n', default_value = "20")]
        limit: i64,
    },
    Approve {
        #[arg(long = "acknowledge-pattern")]
        acknowledge_pattern: Option<String>,
        #[arg(long = "acknowledge-dream-review-token")]
        acknowledge_dream_review_token: Option<String>,
        id: i64,
    },
    Discard {
        id: i64,
    },
    Edit {
        id: i64,
        #[arg(long)]
        text: Option<String>,
        #[arg(long = "topic-key")]
        topic_key: Option<String>,
        #[arg(long = "type")]
        memory_type: Option<String>,
        #[arg(long)]
        scope: Option<String>,
    },
    ApproveBatch {
        #[command(flatten)]
        filter: ReviewBatchFilterArgs,
        #[arg(long)]
        yes: bool,
    },
    DiscardBatch {
        #[command(flatten)]
        filter: ReviewBatchFilterArgs,
        #[arg(long)]
        reason: Option<String>,
        #[arg(long)]
        yes: bool,
    },
    Blocked {
        #[arg(long, short)]
        project: Option<String>,
    },
}

#[derive(clap::Args)]
pub(in crate::cli) struct ReviewBatchFilterArgs {
    #[arg(long, short)]
    pub project: Option<String>,
    #[arg(long = "type")]
    pub memory_type: Option<String>,
    #[arg(long = "block-reason")]
    pub block_reason: Option<String>,
    #[arg(long = "topic-key")]
    pub topic_key: Option<String>,
    #[arg(long)]
    pub contains: Option<String>,
    #[arg(long = "min-confidence")]
    pub min_confidence: Option<f64>,
    #[arg(long = "older-than", value_name = "DAYS")]
    pub older_than_days: Option<i64>,
    #[arg(long, short = 'n', default_value = "200")]
    pub limit: i64,
}

#[derive(Subcommand)]
pub(in crate::cli) enum GraphReviewAction {
    List {
        #[arg(long, short)]
        project: Option<String>,
        #[arg(long, short = 'n', default_value = "20")]
        limit: i64,
    },
    Inspect {
        id: i64,
    },
    Approve {
        id: i64,
    },
    Reject {
        id: i64,
        #[arg(long)]
        reason: String,
    },
    Defer {
        id: i64,
        #[arg(long)]
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::ReviewAction;

    #[derive(Parser)]
    struct ReviewParser {
        #[command(subcommand)]
        action: ReviewAction,
    }

    #[test]
    fn approve_parses_dream_review_token() {
        let parsed = ReviewParser::parse_from([
            "review",
            "approve",
            "--acknowledge-pattern",
            "override_previous_instructions",
            "--acknowledge-dream-review-token",
            "sha256:reviewed",
            "42",
        ]);

        match parsed.action {
            ReviewAction::Approve {
                id,
                acknowledge_pattern,
                acknowledge_dream_review_token,
            } => {
                assert_eq!(id, 42);
                assert_eq!(
                    acknowledge_pattern.as_deref(),
                    Some("override_previous_instructions")
                );
                assert_eq!(
                    acknowledge_dream_review_token.as_deref(),
                    Some("sha256:reviewed")
                );
            }
            _ => panic!("expected approve action"),
        }
    }
}
