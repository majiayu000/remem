/// AI call timeout (seconds)
pub(super) const AI_TIMEOUT_SECS: u64 = 90;

pub struct UsageContext<'a> {
    pub project: Option<&'a str>,
    pub operation: &'a str,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TokenUsage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_creation_tokens: i64,
    pub cache_read_tokens: i64,
    pub raw_input_tokens: i64,
    pub raw_output_tokens: i64,
}

impl TokenUsage {
    pub fn estimated(input_tokens: i64, output_tokens: i64) -> Self {
        Self {
            input_tokens,
            output_tokens,
            raw_input_tokens: input_tokens,
            raw_output_tokens: output_tokens,
            ..Self::default()
        }
    }

    pub fn total_tokens(&self) -> i64 {
        self.input_tokens
            + self.output_tokens
            + self.reasoning_tokens
            + self.cache_creation_tokens
            + self.cache_read_tokens
    }

    pub fn is_empty(&self) -> bool {
        self.total_tokens() == 0
    }

    pub fn add(&mut self, other: &Self) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.reasoning_tokens += other.reasoning_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
        self.raw_input_tokens += other.raw_input_tokens;
        self.raw_output_tokens += other.raw_output_tokens;
    }
}

pub(super) struct AiCallResult {
    pub text: String,
    pub executor: &'static str,
    pub model: String,
    pub usage: Option<TokenUsage>,
    pub usage_source: Option<&'static str>,
}
