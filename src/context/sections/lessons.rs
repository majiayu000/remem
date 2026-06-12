use crate::memory::lesson::LessonMemory;

use super::super::audit::memory_render_metadata;
use super::super::format::{char_len, format_epoch_short, truncate_chars_with_ellipsis};

const PREVIEW_LEN: usize = 180;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(in crate::context) struct LessonRenderSummary {
    pub count: usize,
    pub ids: Vec<i64>,
}

pub(in crate::context) fn render_lessons_with_limit(
    output: &mut String,
    lessons: &[LessonMemory],
    item_limit: usize,
    char_limit: usize,
) -> usize {
    render_lessons_with_summary(output, lessons, item_limit, char_limit).count
}

pub(in crate::context) fn render_lessons_with_summary(
    output: &mut String,
    lessons: &[LessonMemory],
    item_limit: usize,
    char_limit: usize,
) -> LessonRenderSummary {
    if lessons.is_empty() || item_limit == 0 || char_limit == 0 {
        return LessonRenderSummary::default();
    }
    let header = "## Lessons\n";
    let trailer_chars = 1;
    let mut total_chars = char_len(header) + trailer_chars;
    if total_chars >= char_limit {
        return LessonRenderSummary::default();
    }

    let now = chrono::Utc::now().timestamp();
    let mut body = String::new();
    let mut rendered = 0usize;
    let mut ids = Vec::new();
    for lesson in lessons.iter().take(item_limit) {
        let memory = &lesson.memory;
        let metadata = &lesson.metadata;
        let title = format!(
            "**#{} {}** (confidence {:.2}, reinforced {}, {}; {})\n",
            memory.id,
            memory.title,
            metadata.confidence,
            metadata.reinforcement_count,
            format_epoch_short(metadata.last_reinforced_at_epoch),
            memory_render_metadata(memory, now)
        );
        let fixed_chars = char_len(&title) + 1;
        if total_chars + fixed_chars >= char_limit {
            break;
        }
        let preview_limit = (char_limit - total_chars - fixed_chars).min(PREVIEW_LEN);
        let preview = truncate_chars_with_ellipsis(&memory.text, preview_limit);
        if preview.is_empty() {
            continue;
        }
        let item_chars = fixed_chars + char_len(&preview);
        if total_chars + item_chars > char_limit {
            break;
        }
        body.push_str(&title);
        body.push_str(&preview);
        body.push('\n');
        total_chars += item_chars;
        ids.push(memory.id);
        rendered += 1;
    }
    if rendered == 0 {
        return LessonRenderSummary::default();
    }
    output.push_str(header);
    output.push_str(&body);
    output.push('\n');
    LessonRenderSummary {
        count: rendered,
        ids,
    }
}
