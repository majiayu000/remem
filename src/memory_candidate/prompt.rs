use crate::db;
use crate::memory::format::{xml_escape_attr, xml_escape_text};
use crate::memory::MemoryType;

use super::{CandidatePromptPreference, ObservationBatch};

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
Also extract failure trajectories as lesson candidates: compile/test error chains that consumed effort, reverted or abandoned approaches, and repeated corrections on the same topic. Frame the <text> as a preventive guardrail (what failed, why, what to do instead), grounded only in the supplied evidence.
A lesson block may include <outcome>success</outcome> or <outcome>failure</outcome>; use failure for dead ends and reverted work, success for verified working procedures. Omit the field when the outcome is not evidenced.
A block may additionally contain zero or more self-closing <fact subject=\"...\" predicate=\"...\" object=\"...\"/> elements for durable subject-predicate-object relations stated by the evidence. predicate is a closed set: fixed_by, verified_by, supersedes, blocked_by, uses_file, uses_command, affects_project. Only emit a fact when subject and object are concrete evidence-backed identifiers (a file, command, component, issue, or decision); never restate the whole memory text as a fact and never invent relations.
If there is no durable memory candidate, return exactly <no_candidates reason=\"...\"/>.
If evidence is ambiguous or contradictory, return exactly <defer reason=\"...\"/> so it can be retried or reviewed.
Use only provided observations and evidence; do not invent files, outcomes, decisions, or facts.";

pub(super) fn build_candidate_prompt(
    task: &db::ExtractionTask,
    batch: &ObservationBatch,
    existing_preferences: &[CandidatePromptPreference],
) -> String {
    let mut prompt = format!(
        "Task: memory_candidate\nProject: {}\nHost: {}\nSession: {}\nCovered evidence events: {}..{}\n\n",
        task.project,
        task.host,
        task.session_id.as_deref().unwrap_or("<unknown>"),
        batch.from_event_id,
        batch.to_event_id
    );
    append_existing_preferences(&mut prompt, existing_preferences);
    // 单一真实来源：从 MemoryType::ALL 动态生成合法 candidate type 列表注入 prompt，
    // 避免与枚举漂移（曾因 LLM 把 observation type feature/change 抄进 <type> 整批失败）。
    let valid_candidate_types = MemoryType::ALL
        .iter()
        .copied()
        .filter(|memory_type| *memory_type != MemoryType::SessionActivity)
        .map(|memory_type| memory_type.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    prompt.push_str(&format!(
        "Valid candidate <type> values: {valid_candidate_types}.\nDo not copy an observation's type verbatim; observations use a different vocabulary and feature/refactor/change must be mapped to discovery. Factual findings use discovery; never use fact.\n\n"
    ));
    for observation in &batch.observations {
        let evidence = observation
            .evidence_event_ids
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        prompt.push_str(&format!(
            "<observation id=\"{}\" type=\"{}\" confidence=\"{}\" evidence_event_ids=\"{}\">\n",
            observation.id,
            xml_escape_attr(&observation.observation_type),
            observation.confidence,
            xml_escape_attr(&evidence)
        ));
        prompt.push_str(&xml_escape_text(&observation.text));
        prompt.push_str("\n</observation>\n\n");
    }
    prompt
}

fn append_existing_preferences(prompt: &mut String, preferences: &[CandidatePromptPreference]) {
    if preferences.is_empty() {
        return;
    }
    prompt.push_str("<existing_active_preferences>\n");
    prompt.push_str(
        "These preferences are already active for this project. When the current observations provide new evidence of the same correction, emit that preference candidate again so remem can count an evidence-backed reinforcement. Do not emit unsupported restatements or paraphrases. Also emit net-new preferences, material refinements, or explicit contradictions supported by the observations.\n",
    );
    for preference in preferences {
        prompt.push_str(&format!(
            "<preference id=\"{}\">\n",
            xml_escape_attr(&preference.id.to_string())
        ));
        prompt.push_str(&xml_escape_text(&preference.text));
        prompt.push_str("\n</preference>\n");
    }
    prompt.push_str("</existing_active_preferences>\n\n");
}
