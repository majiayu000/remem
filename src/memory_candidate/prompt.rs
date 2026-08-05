pub(super) const MEMORY_CANDIDATE_SYSTEM: &str = "\
Generate durable memory candidates from extracted observations.
Return zero or more <memory_candidate> blocks.
Each block must include <scope>, <type>, <topic_key>, <risk_class>, <confidence>, and <text>.
<type> must be one of the valid candidate memory types listed in the task. Observations use a different type vocabulary (feature/refactor/change are not candidate types), so never copy an observation's type verbatim into <type>; map feature/refactor/change to discovery. Factual findings use discovery; never use fact.
Use scope=project unless the observation is explicitly a stable user preference.
<risk_class> is a closed rubric; use exactly low, medium, or high.
- low: an already-true repository-local claim directly supported by the supplied evidence, including a directly observed failure lesson. Example: a verified retry fixed a specific migration failure.
- medium: a preference, procedure, recommendation, inference, proposal, future plan, or claim whose applicability still needs review. Example: the worker should use a different retry policy.
- high: credentials, authentication or authorization state, private/personal/payment data, destructive operations, or other security-sensitive claims. Example: an access token or permission change.
Negation or engineering words such as fail, skip, pass, and token do not determine risk by themselves; classify the concrete claim. Never emit a risk class outside the closed rubric.
If there is no durable memory candidate, return exactly <no_candidates reason=\"...\"/>.
If evidence is ambiguous or contradictory, return exactly <defer reason=\"...\"/> so it can be retried or reviewed.
Use only provided observations and evidence; do not invent files, outcomes, decisions, or facts.";
