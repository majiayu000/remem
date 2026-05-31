use crate::memory::lesson::{LessonMemory, LessonMetadata};
use crate::workstream::{WorkStream, WorkStreamStatus};

use super::super::host::HostKind;
use super::super::policy::ContextLimits;
use super::super::render::{
    build_context_header, build_context_stats_footer, empty_context_output, ContextRenderStats,
    SectionRenderStats,
};
use super::super::sections::{
    render_core_memory, render_core_memory_with_limits, render_lessons_with_limit,
    render_memory_index, render_memory_index_with_limits,
    render_memory_index_with_limits_excluding, render_recent_sessions,
    render_recent_sessions_with_limit, render_workstreams, render_workstreams_with_limits,
};
use super::super::types::{ContextRequest, SessionSummaryBrief};
use super::{sample_memory, sample_memory_with_epoch, sample_workstream};

#[test]
fn render_recent_sessions_truncates_completed_line() {
    let mut output = String::new();
    let summaries = vec![SessionSummaryBrief {
        request: "Implement feature".to_string(),
        completed: Some(format!("{}\nignored", "x".repeat(130))),
        created_at_epoch: 1_710_000_000,
    }];

    render_recent_sessions(&mut output, &summaries);

    assert!(output.contains("Implement feature"));
    assert!(output.contains("=> "));
    assert!(output.contains("..."));
    assert!(!output.contains("ignored"));
}

#[test]
fn render_recent_sessions_truncates_request_text() {
    let mut output = String::new();
    let long_request = format!("Investigate SessionStart budget {}", "x".repeat(300));
    let summaries = vec![SessionSummaryBrief {
        request: long_request.clone(),
        completed: Some("done".to_string()),
        created_at_epoch: 1_710_000_000,
    }];

    render_recent_sessions(&mut output, &summaries);

    assert!(output.contains("Investigate SessionStart budget"));
    assert!(output.contains("..."));
    assert!(!output.contains(&long_request));
}

#[test]
fn render_recent_sessions_respects_char_limit() {
    let mut output = String::new();
    let summaries = vec![
        SessionSummaryBrief {
            request: "Short followup".to_string(),
            completed: Some("done".to_string()),
            created_at_epoch: 1_710_000_000,
        },
        SessionSummaryBrief {
            request: "Second session should not fit".to_string(),
            completed: Some("done".to_string()),
            created_at_epoch: 1_710_000_100,
        },
    ];

    render_recent_sessions_with_limit(&mut output, &summaries, 70);

    assert!(output.contains("Short followup"));
    assert!(!output.contains("Second session should not fit"));
    assert!(output.chars().count() <= 70);
}

#[test]
fn render_memory_index_prioritizes_known_types() {
    let mut output = String::new();
    let memories = vec![
        sample_memory(1, "custom", "Custom title"),
        sample_memory(2, "bugfix", "Fix title"),
        sample_memory(3, "decision", "Decision title"),
    ];

    render_memory_index(&mut output, &memories);

    let decision_pos = output.find("**Decisions**").unwrap();
    let bugfix_pos = output.find("**Bug Fixes**").unwrap();
    let custom_pos = output.find("**custom**").unwrap();
    assert!(decision_pos < bugfix_pos);
    assert!(bugfix_pos < custom_pos);
}

#[test]
fn render_memory_index_labels_and_orders_procedure_memories() {
    let mut output = String::new();
    let memories = vec![
        sample_memory(1, "session_activity", "Recent session"),
        sample_memory(2, "procedure", "Review procedure"),
        sample_memory(3, "discovery", "Discovery title"),
    ];

    render_memory_index(&mut output, &memories);

    let discovery_pos = output.find("**Discoveries**").unwrap();
    let procedure_pos = output.find("**Procedures**").unwrap();
    let session_pos = output.find("**Sessions**").unwrap();
    assert!(output.contains("Review procedure"));
    assert!(discovery_pos < procedure_pos);
    assert!(procedure_pos < session_pos);
}

#[test]
fn render_memory_index_excludes_preferences() {
    let mut output = String::new();
    let memories = vec![
        sample_memory(1, "preference", "Preference title"),
        sample_memory(2, "decision", "Decision title"),
    ];

    render_memory_index(&mut output, &memories);

    assert!(output.contains("Decision title"));
    assert!(!output.contains("Preference title"));
    assert!(!output.contains("**Preferences**"));
}

#[test]
fn render_memory_index_excludes_lessons() {
    let mut output = String::new();
    let memories = vec![
        sample_memory(1, "lesson", "Lesson title"),
        sample_memory(2, "decision", "Decision title"),
    ];

    render_memory_index(&mut output, &memories);

    assert!(output.contains("Decision title"));
    assert!(!output.contains("Lesson title"));
    assert!(!output.contains("**Lessons**"));
}

#[test]
fn render_lessons_respects_item_and_char_limits() {
    let mut output = String::new();
    let lessons = vec![
        sample_lesson(1, "First lesson", 0.9, 3),
        sample_lesson(2, "Second lesson", 0.8, 1),
    ];

    let rendered = render_lessons_with_limit(&mut output, &lessons, 1, 240);

    assert_eq!(rendered, 1);
    assert!(output.contains("## Lessons"));
    assert!(output.contains("First lesson"));
    assert!(output.contains("reinforced 3"));
    assert!(!output.contains("Second lesson"));
    assert!(output.chars().count() <= 240);
}

#[test]
fn render_memory_index_respects_item_limit() {
    let mut output = String::new();
    let limits = ContextLimits {
        memory_index_limit: 2,
        ..ContextLimits::default()
    };
    let memories = vec![
        sample_memory(1, "decision", "Decision one"),
        sample_memory(2, "decision", "Decision two"),
        sample_memory(3, "decision", "Decision three"),
    ];

    render_memory_index_with_limits(&mut output, &memories, &limits);

    assert!(output.contains("Decision one"));
    assert!(output.contains("Decision two"));
    assert!(!output.contains("Decision three"));
}

#[test]
fn render_memory_index_truncates_first_item_to_char_limit() {
    let mut output = String::new();
    let limits = ContextLimits {
        memory_index_char_limit: 48,
        ..ContextLimits::default()
    };
    let long_title = "Decision title that is far too long for the index budget";
    let memories = vec![sample_memory(1, "decision", long_title)];

    let rendered = render_memory_index_with_limits(&mut output, &memories, &limits);
    let body = output.strip_prefix("## Index\n").unwrap().trim_end();

    assert_eq!(rendered, 1);
    assert!(body.chars().count() <= limits.memory_index_char_limit);
    assert!(output.contains("..."));
    assert!(!output.contains(long_title));
}

#[test]
fn render_memory_index_can_skip_core_selected_ids() {
    let mut core_output = String::new();
    let now = chrono::Utc::now().timestamp();
    let memories = vec![
        sample_memory_with_epoch(1, "bugfix", "Core bugfix", now),
        sample_memory_with_epoch(2, "decision", "Index decision", now),
    ];
    let core_summary = render_core_memory_with_limits(
        &mut core_output,
        &memories,
        &ContextLimits {
            core_item_limit: 1,
            ..ContextLimits::default()
        },
    );
    let excluded_ids = core_summary.ids.into_iter().collect();

    let mut index_output = String::new();
    let rendered = render_memory_index_with_limits_excluding(
        &mut index_output,
        &memories,
        &ContextLimits::default(),
        &excluded_ids,
    );

    assert_eq!(rendered, 1);
    assert!(!index_output.contains("Core bugfix"));
    assert!(index_output.contains("Index decision"));
}

#[test]
fn render_workstreams_includes_next_action_when_present() {
    let mut output = String::new();
    let workstreams = vec![WorkStream {
        id: 7,
        project: "demo/project".to_string(),
        title: "Refactor context".to_string(),
        description: None,
        status: WorkStreamStatus::Active,
        progress: None,
        next_action: Some("split renderers".to_string()),
        blockers: None,
        created_at_epoch: 0,
        updated_at_epoch: 0,
        completed_at_epoch: None,
    }];

    render_workstreams(&mut output, &workstreams);

    assert!(output.contains("#7 [active] Refactor context -> split renderers"));
}

#[test]
fn render_workstreams_respects_item_and_char_limits() {
    let mut output = String::new();
    let workstreams = vec![
        sample_workstream(1, "First stream", Some("ship the first fix")),
        sample_workstream(2, "Second stream", Some("ship the second fix")),
        sample_workstream(3, "Third stream", Some("ship the third fix")),
    ];

    render_workstreams_with_limits(&mut output, &workstreams, 2, 200);

    assert!(output.contains("#1 [active] First stream"));
    assert!(output.contains("#2 [active] Second stream"));
    assert!(!output.contains("#3 [active] Third stream"));
    assert!(output.chars().count() <= 200);
}

#[test]
fn render_workstreams_stops_at_char_limit() {
    let mut output = String::new();
    let workstreams = vec![
        sample_workstream(1, "First", Some("fix")),
        sample_workstream(2, "Second", Some("fix")),
    ];

    render_workstreams_with_limits(&mut output, &workstreams, 10, 48);

    assert!(output.contains("#1 [active] First"));
    assert!(!output.contains("#2 [active] Second"));
    assert!(output.chars().count() <= 48);
}

#[test]
fn render_core_memory_prioritizes_higher_score_memories() {
    let mut output = String::new();
    let now = chrono::Utc::now().timestamp();
    let memories = vec![
        sample_memory_with_epoch(1, "discovery", "Lower score", now),
        sample_memory_with_epoch(2, "decision", "Higher score", now),
    ];

    render_core_memory(&mut output, &memories);

    let high_pos = output.find("**#2 Higher score**").unwrap();
    let low_pos = output.find("**#1 Lower score**").unwrap();
    assert!(high_pos < low_pos);
}

#[test]
fn render_core_memory_truncates_first_item_to_char_limit() {
    let mut output = String::new();
    let limits = ContextLimits {
        core_char_limit: 120,
        ..ContextLimits::default()
    };
    let mut long_memory = sample_memory(1, "decision", "Compact title");
    long_memory.text = "x".repeat(500);
    let memories = vec![long_memory];

    render_core_memory_with_limits(&mut output, &memories, &limits);

    let body = output.strip_prefix("## Core\n").unwrap().trim_end();
    assert!(output.chars().count() <= limits.core_char_limit);
    assert!(body.chars().count() <= limits.core_char_limit);
    assert!(output.contains("..."));
}

#[test]
fn render_core_memory_keeps_type_diversity_before_filling_same_type() {
    let mut output = String::new();
    let now = chrono::Utc::now().timestamp();
    let memories = vec![
        sample_memory_with_epoch(1, "decision", "Decision one", now),
        sample_memory_with_epoch(2, "decision", "Decision two", now),
        sample_memory_with_epoch(3, "decision", "Decision three", now),
        sample_memory_with_epoch(4, "discovery", "Operational discovery", now),
    ];

    render_core_memory(&mut output, &memories);

    let discovery_pos = output.find("**#4 Operational discovery**").unwrap();
    let third_decision_pos = output.find("**#3 Decision three**").unwrap();
    assert!(discovery_pos < third_decision_pos);
}

#[test]
fn render_core_memory_does_not_backfill_with_memory_self_diagnostics() {
    let mut output = String::new();
    let now = chrono::Utc::now().timestamp();
    let memories = vec![
        sample_memory_with_epoch(1, "decision", "Memory injection diagnosis", now),
        sample_memory_with_epoch(2, "discovery", "Runtime hook finding", now),
    ];

    render_core_memory(&mut output, &memories);

    let runtime_pos = output.find("**#2 Runtime hook finding**").unwrap();
    assert!(runtime_pos < output.len());
    assert!(!output.contains("Memory injection diagnosis"));
}

#[test]
fn render_core_memory_keeps_stale_decision_out_when_recent_context_is_available() {
    let mut output = String::new();
    let now = chrono::Utc::now().timestamp();
    let stale_epoch = now - 8 * 86400;
    let memories = vec![
        sample_memory_with_epoch(1, "decision", "Recent decision one", now),
        sample_memory_with_epoch(3, "discovery", "Recent discovery one", now),
        sample_memory_with_epoch(4, "discovery", "Recent discovery two", now),
        sample_memory_with_epoch(5, "preference", "Recent preference one", now),
        sample_memory_with_epoch(6, "preference", "Recent preference two", now),
        sample_memory_with_epoch(7, "decision", "Stale landing page decision", stale_epoch),
    ];

    render_core_memory(&mut output, &memories);

    assert!(!output.contains("Stale landing page decision"));
}

#[test]
fn context_stats_footer_reports_budget_scope_and_truncation() {
    let footer = build_context_stats_footer(&ContextRenderStats {
        host: "codex-cli".to_string(),
        branch: Some("fix/context".to_string()),
        hook_source: Some("compact".to_string()),
        total_char_limit: 12_000,
        memories_loaded: 7,
        core: SectionRenderStats {
            count: 2,
            chars: 430,
        },
        lessons: SectionRenderStats {
            count: 1,
            chars: 180,
        },
        index: SectionRenderStats {
            count: 5,
            chars: 800,
        },
        preferences: SectionRenderStats {
            count: 3,
            chars: 240,
        },
        project_preferences: 2,
        global_preferences: 1,
        sessions: SectionRenderStats {
            count: 4,
            chars: 620,
        },
        workstreams: SectionRenderStats {
            count: 1,
            chars: 80,
        },
        owner_counts: Default::default(),
        core_ids: vec![1, 2],
        output_chars: 3_200,
        truncated: true,
    });

    assert!(footer.contains("7 context memories loaded"));
    assert!(footer.contains("2 core (430 chars)"));
    assert!(footer.contains("1 lessons (180 chars)"));
    assert!(footer.contains("3 preferences (project:2 global:1"));
    assert!(footer.contains("host=codex-cli"));
    assert!(footer.contains("source=compact"));
    assert!(footer.contains("branch=fix/context"));
    assert!(footer.contains("total=3200 chars/~800 tokens"));
    assert!(footer.contains("truncated=yes"));
}

#[test]
fn context_header_marks_compact_reload_visibly() {
    let header = build_context_header("/tmp/remem", Some("main"), Some("compact"));

    assert!(header.starts_with("# [/tmp/remem @main] context "));
    assert!(header.contains("[REMEM POST-COMPACT RELOAD]"));
}

#[test]
fn empty_context_marks_compact_reload_visibly() {
    let output = empty_context_output(&ContextRequest {
        cwd: "/tmp/remem".to_string(),
        project: "/tmp/remem".to_string(),
        session_id: None,
        hook_source: Some("compact".to_string()),
        current_branch: Some("main".to_string()),
        host: HostKind::CodexCli,
        use_colors: false,
    });

    assert!(output.starts_with("# [/tmp/remem @main] context "));
    assert!(output.contains("[REMEM POST-COMPACT RELOAD]"));
    assert!(output.contains("REMEM_CONTEXT_SOURCE=compact"));
    assert!(output.contains("No previous sessions found."));
}

fn sample_lesson(id: i64, title: &str, confidence: f64, reinforcement_count: i64) -> LessonMemory {
    LessonMemory {
        memory: sample_memory(id, "lesson", title),
        metadata: LessonMetadata {
            memory_id: id,
            confidence,
            reinforcement_count,
            source_evidence: None,
            last_reinforced_at_epoch: 1_710_000_000,
            stale_after_epoch: None,
        },
    }
}
