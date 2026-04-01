/// AI call timeout (seconds)
pub(super) const AI_TIMEOUT_SECS: u64 = 90;

pub struct UsageContext<'a> {
    pub project: Option<&'a str>,
    pub operation: &'a str,
}

pub(super) struct AiCallResult {
    pub text: String,
    pub executor: &'static str,
    pub model: String,
}
