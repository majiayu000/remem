use anyhow::Result;
use chrono::{FixedOffset, Local, TimeZone};
use std::sync::OnceLock;

pub const LABEL_SEPARATOR: &str = "｜";
pub const TOPIC_MAX_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIntent {
    Fea,
    Des,
    Fix,
    Opt,
    Rel,
    Exp,
    Doc,
    Res,
}

impl SessionIntent {
    pub const ALL: [Self; 8] = [
        Self::Fea,
        Self::Des,
        Self::Fix,
        Self::Opt,
        Self::Rel,
        Self::Exp,
        Self::Doc,
        Self::Res,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fea => "fea",
            Self::Des => "des",
            Self::Fix => "fix",
            Self::Opt => "opt",
            Self::Rel => "rel",
            Self::Exp => "exp",
            Self::Doc => "doc",
            Self::Res => "res",
        }
    }

    pub const fn chinese_label(self) -> &'static str {
        match self {
            Self::Fea => "功能",
            Self::Des => "设计",
            Self::Fix => "修复",
            Self::Opt => "优化",
            Self::Rel => "发布",
            Self::Exp => "探索",
            Self::Doc => "文档",
            Self::Res => "调研",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        let normalized = value.trim().to_ascii_lowercase();
        Self::ALL
            .iter()
            .copied()
            .find(|intent| intent.as_str() == normalized)
    }

    /// Fail-closed writer used by #1067/#1068; listing only parses stored values.
    #[allow(dead_code)]
    pub fn parse_write(value: &str) -> Result<Self> {
        Self::parse(value).ok_or_else(|| {
            anyhow::anyhow!("unknown session_intent '{value}'; expected a closed v1 code")
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionIntentSource {
    Summary,
    Override,
    Rollup,
}

impl SessionIntentSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Summary => "summary",
            Self::Override => "override",
            Self::Rollup => "rollup",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "summary" => Some(Self::Summary),
            "override" => Some(Self::Override),
            "rollup" => Some(Self::Rollup),
            _ => None,
        }
    }

    #[allow(dead_code)]
    pub fn parse_write(value: &str) -> Result<Self> {
        Self::parse(value).ok_or_else(|| {
            anyhow::anyhow!(
                "unknown session_intent_source '{value}'; expected summary, override, or rollup"
            )
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentLanguage {
    English,
    #[allow(dead_code)]
    Chinese,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionLabelView {
    pub mmdd: Option<String>,
    pub session_intent: Option<String>,
    pub session_topic: Option<String>,
    pub display_label: Option<String>,
    pub session_intent_source: Option<String>,
    pub display_title: String,
}

/// Calendar zone used to derive `MMDD` from a session's created epoch.
/// `MMDD` is never stored, so changing the zone needs no backfill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LabelTimeZone {
    /// The machine's local zone (honors `TZ` on Unix), including DST.
    Local,
    Fixed(FixedOffset),
}

impl LabelTimeZone {
    /// Accepts `local`, `UTC`/`Z`, or a fixed offset such as `+08:00`,
    /// `-0530`, or `+09`.
    pub fn parse(raw: &str) -> Result<Self> {
        let value = raw.trim();
        if value.eq_ignore_ascii_case("local") {
            return Ok(Self::Local);
        }
        if value.eq_ignore_ascii_case("utc") || value.eq_ignore_ascii_case("z") {
            return Ok(Self::Fixed(FixedOffset::east_opt(0).expect("UTC is valid")));
        }
        parse_fixed_offset(value).map(Self::Fixed).ok_or_else(|| {
            anyhow::anyhow!(
                "invalid session_labels.timezone '{value}'; expected local, UTC, or a fixed offset such as +08:00"
            )
        })
    }

    pub fn mmdd(self, epoch: i64) -> Option<String> {
        let when = match self {
            Self::Local => Local.timestamp_opt(epoch, 0).single()?.fixed_offset(),
            Self::Fixed(offset) => offset.timestamp_opt(epoch, 0).single()?,
        };
        Some(when.format("%m%d").to_string())
    }
}

fn parse_fixed_offset(value: &str) -> Option<FixedOffset> {
    let (sign, rest) = match value.as_bytes().first()? {
        b'+' => (1, &value[1..]),
        b'-' => (-1, &value[1..]),
        _ => return None,
    };
    let digits = rest.replace(':', "");
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) || !matches!(digits.len(), 2 | 4) {
        return None;
    }
    let hours: i32 = digits[..2].parse().ok()?;
    let minutes: i32 = if digits.len() == 4 {
        digits[2..].parse().ok()?
    } else {
        0
    };
    if hours > 14 || minutes > 59 {
        return None;
    }
    FixedOffset::east_opt(sign * (hours * 3600 + minutes * 60))
}

/// Resolved once per process from `REMEM_SESSION_LABEL_TIMEZONE` or
/// `session_labels.timezone`; invalid values fall back to `local`.
pub fn label_timezone() -> LabelTimeZone {
    static ZONE: OnceLock<LabelTimeZone> = OnceLock::new();
    *ZONE.get_or_init(resolve_label_timezone)
}

#[cfg(not(test))]
fn resolve_label_timezone() -> LabelTimeZone {
    let resolved = crate::runtime_config::session_labels::session_label_timezone_setting()
        .and_then(|setting| LabelTimeZone::parse(&setting));
    match resolved {
        Ok(zone) => zone,
        Err(error) => {
            crate::log::warn(
                "session-label",
                &format!("timezone fallback=local error={error}"),
            );
            LabelTimeZone::Local
        }
    }
}

/// Unit tests pin UTC+8 so label fixtures do not depend on the machine zone;
/// resolution itself is covered by `runtime_config::session_labels` tests.
#[cfg(test)]
fn resolve_label_timezone() -> LabelTimeZone {
    LabelTimeZone::Fixed(FixedOffset::east_opt(8 * 3600).expect("UTC+8 is valid"))
}

pub fn mmdd_from_created_epoch(epoch: i64) -> Option<String> {
    label_timezone().mmdd(epoch)
}

pub fn normalize_topic(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().count() > TOPIC_MAX_CHARS {
        return None;
    }
    Some(trimmed.to_string())
}

pub fn render_session_label(
    created_at_epoch: Option<i64>,
    intent: Option<SessionIntent>,
    topic: Option<&str>,
    source: Option<SessionIntentSource>,
    fallback_title: Option<&str>,
    language: IntentLanguage,
) -> SessionLabelView {
    let mmdd = created_at_epoch.and_then(mmdd_from_created_epoch);
    let session_topic = topic.and_then(normalize_topic);
    let session_intent = intent.map(|value| match language {
        IntentLanguage::English => value.as_str().to_string(),
        IntentLanguage::Chinese => value.chinese_label().to_string(),
    });
    let display_label = match (&mmdd, &session_intent, &session_topic) {
        (Some(mmdd), Some(intent), Some(topic)) => Some(format!(
            "{mmdd}{LABEL_SEPARATOR}{intent}{LABEL_SEPARATOR}{topic}"
        )),
        _ => None,
    };
    let display_title = display_label
        .clone()
        .or_else(|| {
            fallback_title
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_default();
    SessionLabelView {
        mmdd,
        session_intent,
        session_topic,
        display_label,
        session_intent_source: source.map(SessionIntentSource::as_str).map(str::to_string),
        display_title,
    }
}

pub fn render_from_stored(
    created_at_epoch: Option<i64>,
    intent: Option<&str>,
    topic: Option<&str>,
    source: Option<&str>,
    fallback_title: Option<&str>,
) -> SessionLabelView {
    render_session_label(
        created_at_epoch,
        intent.and_then(SessionIntent::parse),
        topic,
        source.and_then(SessionIntentSource::parse),
        fallback_title,
        IntentLanguage::English,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed(raw: &str) -> LabelTimeZone {
        LabelTimeZone::parse(raw).expect("valid offset")
    }

    #[test]
    fn timezone_parse_accepts_local_utc_and_fixed_offsets() {
        assert_eq!(
            LabelTimeZone::parse(" Local ").unwrap(),
            LabelTimeZone::Local
        );
        let utc = LabelTimeZone::Fixed(FixedOffset::east_opt(0).unwrap());
        assert_eq!(LabelTimeZone::parse("UTC").unwrap(), utc);
        assert_eq!(LabelTimeZone::parse("z").unwrap(), utc);
        let plus8 = LabelTimeZone::Fixed(FixedOffset::east_opt(8 * 3600).unwrap());
        assert_eq!(fixed("+08:00"), plus8);
        assert_eq!(fixed("+0800"), plus8);
        assert_eq!(fixed("+08"), plus8);
        assert_eq!(
            fixed("-05:30"),
            LabelTimeZone::Fixed(FixedOffset::west_opt(5 * 3600 + 30 * 60).unwrap())
        );
    }

    #[test]
    fn timezone_parse_rejects_names_and_out_of_range_offsets() {
        for raw in [
            "Asia/Shanghai",
            "",
            "08:00",
            "+8",
            "+15:00",
            "+08:60",
            "+08:0a",
        ] {
            let error = LabelTimeZone::parse(raw).unwrap_err().to_string();
            assert!(error.contains("session_labels.timezone"), "{raw}: {error}");
        }
    }

    #[test]
    fn mmdd_date_boundary_follows_the_configured_zone() {
        // 2024-12-31 16:00:00 UTC: already 01-01 at UTC+8, still 12-31 in UTC.
        let new_year_plus8 = 1_735_660_800;
        assert_eq!(
            fixed("+08:00").mmdd(new_year_plus8).as_deref(),
            Some("0101")
        );
        assert_eq!(
            fixed("+08:00").mmdd(new_year_plus8 - 1).as_deref(),
            Some("1231")
        );
        assert_eq!(fixed("UTC").mmdd(new_year_plus8).as_deref(), Some("1231"));
        // 2025-01-01 00:00:00 UTC is 2024-12-31 19:00 at UTC-05:00.
        let new_year_utc = 1_735_689_600;
        assert_eq!(fixed("UTC").mmdd(new_year_utc).as_deref(), Some("0101"));
        assert_eq!(fixed("-05:00").mmdd(new_year_utc).as_deref(), Some("1231"));
    }

    #[test]
    fn local_zone_matches_the_process_local_calendar() {
        let epoch = 1_735_660_800;
        let expected = Local
            .timestamp_opt(epoch, 0)
            .single()
            .map(|when| when.format("%m%d").to_string());
        assert_eq!(LabelTimeZone::Local.mmdd(epoch), expected);
    }

    #[test]
    fn unit_tests_pin_label_zone_to_utc_plus_8() {
        assert_eq!(label_timezone(), fixed("+08:00"));
        assert_eq!(
            mmdd_from_created_epoch(1_735_660_800).as_deref(),
            Some("0101")
        );
    }

    #[test]
    fn closed_intent_enum_parses_and_rejects_unknown_writes() {
        assert_eq!(SessionIntent::parse("FIX"), Some(SessionIntent::Fix));
        assert_eq!(SessionIntent::parse(" bug "), None);
        assert!(SessionIntent::parse_write("fea").is_ok());
        let error = SessionIntent::parse_write("bugfix")
            .unwrap_err()
            .to_string();
        assert!(error.contains("unknown session_intent"));
    }

    #[test]
    fn unknown_stored_intent_abstains_instead_of_rendering_a_label() {
        let view = render_from_stored(
            Some(1_735_660_800),
            Some("bugfix"),
            Some("Batch text display"),
            Some("summary"),
            Some("fallback title"),
        );
        assert_eq!(view.mmdd.as_deref(), Some("0101"));
        assert_eq!(view.session_intent, None);
        assert_eq!(view.display_label, None);
        assert_eq!(view.display_title, "fallback title");
    }

    #[test]
    fn missing_intent_or_topic_abstains_from_the_full_label() {
        let missing_topic = render_session_label(
            Some(1_735_660_800),
            Some(SessionIntent::Fix),
            None,
            Some(SessionIntentSource::Summary),
            Some("Repair listing"),
            IntentLanguage::English,
        );
        assert_eq!(missing_topic.display_label, None);
        assert_eq!(missing_topic.display_title, "Repair listing");

        let missing_intent = render_session_label(
            Some(1_735_660_800),
            None,
            Some("Batch text display"),
            None,
            Some("Repair listing"),
            IntentLanguage::English,
        );
        assert_eq!(missing_intent.display_label, None);
    }

    #[test]
    fn present_intent_and_topic_render_fullwidth_label() {
        let view = render_session_label(
            Some(1_735_660_800),
            Some(SessionIntent::Fix),
            Some("  Batch text display  "),
            Some(SessionIntentSource::Summary),
            Some("ignored fallback"),
            IntentLanguage::English,
        );
        assert_eq!(
            view.display_label.as_deref(),
            Some("0101｜fix｜Batch text display")
        );
        assert!(view
            .display_label
            .as_ref()
            .unwrap()
            .contains(LABEL_SEPARATOR));
        assert_eq!(view.session_intent.as_deref(), Some("fix"));
        assert_eq!(view.session_intent_source.as_deref(), Some("summary"));
        assert_eq!(view.display_title, "0101｜fix｜Batch text display");
    }

    #[test]
    fn chinese_labels_are_display_only_and_do_not_mix_in_english_mode() {
        let chinese = render_session_label(
            Some(1_735_660_800),
            Some(SessionIntent::Fix),
            Some("批量文本展示"),
            None,
            None,
            IntentLanguage::Chinese,
        );
        assert_eq!(
            chinese.display_label.as_deref(),
            Some("0101｜修复｜批量文本展示")
        );
        let english = render_session_label(
            Some(1_735_660_800),
            Some(SessionIntent::Fix),
            Some("批量文本展示"),
            None,
            None,
            IntentLanguage::English,
        );
        assert_eq!(english.session_intent.as_deref(), Some("fix"));
    }

    #[test]
    fn topic_too_long_or_blank_abstains() {
        assert_eq!(normalize_topic("  "), None);
        let too_long = "n".repeat(TOPIC_MAX_CHARS + 1);
        assert_eq!(normalize_topic(&too_long), None);
        assert_eq!(
            normalize_topic(&"n".repeat(TOPIC_MAX_CHARS)).as_deref(),
            Some(&*"n".repeat(TOPIC_MAX_CHARS))
        );
    }

    #[test]
    fn source_write_is_fail_closed() {
        assert_eq!(
            SessionIntentSource::parse_write("override").unwrap(),
            SessionIntentSource::Override
        );
        assert!(SessionIntentSource::parse_write("manual").is_err());
        assert_eq!(SessionIntentSource::parse("legacy"), None);
    }
}
