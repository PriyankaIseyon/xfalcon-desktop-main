//! Maps DA report kits to the contract's report views, and checks the inputs
//! every report operation takes before any request leaves the device.
//!
//! Pure: nothing here touches the network, so every rule is unit-tested
//! directly.

use xfalcon_contracts::reports::{
    ReportErrorCode, ReportId, ReportKind, ReportOperationError, ReportScope, ReportView,
};
use xfalcon_contracts::{ConversationId, CONTRACT_VERSION};

use crate::clients::da::generated::{KitSummaryResponse, ResponseKind};
use crate::clients::da::kits_api::DaKitsApiError;

/// The longest name DA accepts is 255; the surface bounds a name to 200 so a
/// rename cannot fail on DA's limit after the surface said it was fine.
const MAX_TITLE_CHARS: usize = 200;
const MAX_DESCRIPTION_CHARS: usize = 4000;
/// A page name is a query value; anything longer is not a page DA stored.
const MAX_PAGE_NAME_BYTES: usize = 256;

/// The view of one library kit, of any kind: a dashboard, a workbook, or a deck.
pub(crate) fn kit_to_view(kit: &KitSummaryResponse) -> Option<ReportView> {
    // A kit with no usable id (the DTO fills an absent one with an empty string)
    // cannot be opened, renamed, or deleted, so it is not listed.
    if !valid_kit_id(&kit.kit_id) {
        return None;
    }
    Some(ReportView {
        version: CONTRACT_VERSION,
        id: ReportId::from(kit.kit_id.clone()),
        kind: match kit.kind {
            ResponseKind::Dashboard => ReportKind::Dashboard,
            ResponseKind::Workbook => ReportKind::Workbook,
            ResponseKind::Presentation => ReportKind::Presentation,
        },
        title: kit.name.clone(),
        description: kit.description.clone(),
        created_at_ms: timestamp_ms(kit.created_at.as_deref()),
        updated_at_ms: timestamp_ms(kit.updated_at.as_deref()),
        // Absent rather than zero when unparseable: a card asks for an image only
        // when DA said it holds one.
        thumbnail_updated_at_ms: kit
            .thumbnail_updated_at
            .as_deref()
            .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
            .map(|parsed| parsed.timestamp_millis()),
        pinned: kit.pinned.unwrap_or(false),
        hidden: kit.hidden.unwrap_or(false),
        byte_size: kit.total_payload_bytes.max(0) as u64,
        scope: if kit.scope == "global" {
            ReportScope::Global
        } else {
            ReportScope::User
        },
        is_owner: kit.is_owner,
        owner_display_name: kit.owner_display_name.clone(),
        refreshable: kit.refreshable,
        can_refresh: kit.can_refresh.unwrap_or(false),
        can_manage: kit.can_manage.unwrap_or(false),
        can_delete: kit.can_delete.unwrap_or(false),
        page_count: clamp_count(kit.page_count),
        dataset_count: clamp_count(kit.dataset_count),
        data_sources: data_sources(kit.data_sources.as_deref().unwrap_or_default()),
        source_conversation_id: kit.source_conversation_id.clone().map(ConversationId::from),
    })
}

/// The most data-source names a card is given, and the longest each may be.
/// DA lists a kit's distinct sources, so a real kit is far inside both; the bound
/// keeps a garbled answer from filling a card.
const MAX_DATA_SOURCES: usize = 20;
const MAX_DATA_SOURCE_CHARS: usize = 200;

/// DA's data-source names, trimmed, deduplicated in order, one line each, and
/// bounded. A name that is empty or unprintable is dropped rather than shown.
fn data_sources(names: &[String]) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for name in names {
        let name = name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_DATA_SOURCE_CHARS
            || name.chars().any(is_unprintable)
            || kept.iter().any(|seen| seen == name)
        {
            continue;
        }
        kept.push(name.to_owned());
        if kept.len() == MAX_DATA_SOURCES {
            break;
        }
    }
    kept
}

/// RFC 3339 to Unix milliseconds. Zero when the value is absent or unparseable:
/// the list shows no date rather than failing the whole library over one field.
fn timestamp_ms(value: Option<&str>) -> i64 {
    value
        .and_then(|text| chrono::DateTime::parse_from_rfc3339(text).ok())
        .map_or(0, |parsed| parsed.timestamp_millis())
}

fn clamp_count(value: i64) -> u32 {
    u32::try_from(value.clamp(0, i64::from(u32::MAX))).unwrap_or(u32::MAX)
}

/// A character that has no place in a one-line name: a control character, a
/// line or paragraph separator, a bidirectional override, embedding or isolate
/// (which can make a name read as another), or an invisible character that
/// shows as nothing (zero-width space, word joiner, byte-order mark).
/// `char::is_control` alone covers only the first of these. The zero-width
/// joiner and non-joiner and the left/right marks stay allowed: emoji
/// sequences and Persian or Indic names need them.
fn is_unprintable(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200B}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{FEFF}'
        )
}

/// A name the reader typed: trimmed, required, bounded, and one line of
/// printable text (a newline would break the card it is shown on).
pub(crate) fn clean_title(title: &str) -> Result<String, ReportOperationError> {
    let trimmed = title.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > MAX_TITLE_CHARS
        || trimmed.chars().any(is_unprintable)
    {
        return Err(ReportOperationError::new(ReportErrorCode::InvalidTitle));
    }
    Ok(trimmed.to_string())
}

/// A description is optional. Empty after trimming means "no description", and
/// the caller sends that as an explicit null so a cleared description is cleared.
/// One past DA's bound is refused, not cut: a description that silently loses
/// its tail is a worse answer than saying it is too long.
pub(crate) fn clean_description(
    description: Option<&str>,
) -> Result<Option<String>, ReportOperationError> {
    let Some(trimmed) = description.map(str::trim).filter(|text| !text.is_empty()) else {
        return Ok(None);
    };
    if trimmed.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(ReportOperationError::new(ReportErrorCode::InvalidTitle));
    }
    Ok(Some(trimmed.to_string()))
}

/// Whether a report id can be a DA kit id: DA mints them as UUIDs. An id that
/// fails this is answered exactly as an unknown one, so the answer never reveals
/// which ids are real, and nothing but a UUID is ever placed in a request path
/// (the literal segments `.` and `..` would otherwise be dropped by the URL
/// builder and name a different route).
pub(crate) fn valid_kit_id(id: &str) -> bool {
    id.len() == 36 && uuid::Uuid::parse_str(id).is_ok()
}

/// Whether a page name can be one DA stored: non-empty, bounded, printable.
pub(crate) fn valid_page_name(page: &str) -> bool {
    !page.is_empty() && page.len() <= MAX_PAGE_NAME_BYTES && !page.chars().any(char::is_control)
}

/// Maps a kit-client error to the contract. Only the cases the reader can act on
/// get their own code; a 401, 409, 413, 422, or transport failure reads as
/// storage. A 422 means something different for each operation (an invalid name
/// on a rename, no stored queries on a refresh), so the operations that can
/// say what it meant map it themselves before they get here.
pub(crate) fn map_kits_error(err: DaKitsApiError) -> ReportOperationError {
    let code = match err {
        DaKitsApiError::NotFound => ReportErrorCode::NotFound,
        DaKitsApiError::Forbidden => ReportErrorCode::Forbidden,
        DaKitsApiError::Unauthorized
        | DaKitsApiError::Conflict
        | DaKitsApiError::PayloadTooLarge
        | DaKitsApiError::Validation
        | DaKitsApiError::RequestFailed
        | DaKitsApiError::InvalidResponse => ReportErrorCode::StorageUnavailable,
    };
    ReportOperationError::new(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    const KIT: &str = "6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01";

    fn kit(kind: ResponseKind) -> KitSummaryResponse {
        KitSummaryResponse {
            kit_id: KIT.to_string(),
            name: "Pipeline".to_string(),
            kind,
            scope: "user".to_string(),
            owner_user_id: "user-1".to_string(),
            owner_display_name: None,
            is_owner: true,
            refreshable: true,
            can_refresh: Some(true),
            can_manage: Some(true),
            can_edit: Some(false),
            can_delete: Some(true),
            source_artifact_id: None,
            source_conversation_id: None,
            page_count: 2,
            dataset_count: 3,
            total_payload_bytes: 512,
            created_at: Some("2026-10-01T10:00:00Z".to_string()),
            updated_at: Some("2026-10-02T10:00:00.5+00:00".to_string()),
            description: Some("Weekly".to_string()),
            pinned: Some(true),
            hidden: Some(false),
            thumbnail_updated_at: None,
            data_sources: None,
        }
    }

    #[test]
    fn every_kind_of_kit_is_a_report_and_keeps_its_kind() {
        for (answered, kind) in [
            (ResponseKind::Dashboard, ReportKind::Dashboard),
            (ResponseKind::Workbook, ReportKind::Workbook),
            (ResponseKind::Presentation, ReportKind::Presentation),
        ] {
            let view = kit_to_view(&kit(answered)).expect("every kind is listed");
            assert_eq!(view.kind, kind);
        }
    }

    #[test]
    fn a_kit_the_reader_hid_says_so() {
        let mut hidden = kit(ResponseKind::Dashboard);
        hidden.hidden = Some(true);
        assert!(kit_to_view(&hidden).expect("hidden kits are listed").hidden);
        // A DA that sends no flag has hidden nothing.
        hidden.hidden = None;
        assert!(!kit_to_view(&hidden).expect("listed").hidden);
    }

    #[test]
    fn a_thumbnail_time_is_carried_and_an_absent_or_unreadable_one_is_none() {
        let mut thumbed = kit(ResponseKind::Dashboard);
        assert_eq!(
            kit_to_view(&thumbed)
                .expect("listed")
                .thumbnail_updated_at_ms,
            None
        );
        thumbed.thumbnail_updated_at = Some("2026-10-02T10:00:01Z".to_string());
        assert_eq!(
            kit_to_view(&thumbed)
                .expect("listed")
                .thumbnail_updated_at_ms,
            Some(1_790_935_201_000)
        );
        thumbed.thumbnail_updated_at = Some("yesterday".to_string());
        assert_eq!(
            kit_to_view(&thumbed)
                .expect("listed")
                .thumbnail_updated_at_ms,
            None
        );
    }

    #[test]
    fn data_sources_are_carried_clean_and_bounded() {
        let mut sourced = kit(ResponseKind::Dashboard);
        assert!(kit_to_view(&sourced)
            .expect("listed")
            .data_sources
            .is_empty());
        sourced.data_sources = Some(vec![
            " Sales warehouse ".to_string(),
            "Sales warehouse".to_string(),
            String::new(),
            "bad\nname".to_string(),
            "x".repeat(MAX_DATA_SOURCE_CHARS + 1),
            "Finance".to_string(),
        ]);
        assert_eq!(
            kit_to_view(&sourced).expect("listed").data_sources,
            ["Sales warehouse", "Finance"]
        );
        sourced.data_sources = Some((0..50).map(|i| format!("source {i}")).collect());
        assert_eq!(
            kit_to_view(&sourced).expect("listed").data_sources.len(),
            MAX_DATA_SOURCES
        );
    }

    #[test]
    fn a_dashboard_kit_maps_field_by_field() {
        let view = kit_to_view(&kit(ResponseKind::Dashboard)).expect("dashboard");
        assert_eq!(view.id.as_str(), KIT);
        assert_eq!(view.title, "Pipeline");
        assert_eq!(view.description.as_deref(), Some("Weekly"));
        assert!(view.pinned);
        assert_eq!(view.byte_size, 512);
        assert_eq!(view.scope, ReportScope::User);
        assert!(view.is_owner);
        assert!(view.refreshable);
        assert!(view.can_refresh && view.can_manage && view.can_delete);
        assert_eq!(view.page_count, 2);
        assert_eq!(view.dataset_count, 3);
        assert!(view.source_conversation_id.is_none());
    }

    #[test]
    fn absent_capability_flags_read_as_false() {
        let mut summary = kit(ResponseKind::Dashboard);
        summary.can_refresh = None;
        summary.can_manage = None;
        summary.can_delete = None;
        summary.pinned = None;
        let view = kit_to_view(&summary).expect("dashboard");
        assert!(!view.can_refresh && !view.can_manage && !view.can_delete);
        assert!(!view.pinned);
    }

    #[test]
    fn a_global_scope_is_global_and_anything_else_is_user() {
        let mut summary = kit(ResponseKind::Dashboard);
        summary.scope = "global".to_string();
        assert_eq!(
            kit_to_view(&summary).expect("dashboard").scope,
            ReportScope::Global
        );
        summary.scope = "something-new".to_string();
        assert_eq!(
            kit_to_view(&summary).expect("dashboard").scope,
            ReportScope::User
        );
    }

    #[test]
    fn timestamps_parse_to_milliseconds_and_default_to_zero() {
        let view = kit_to_view(&kit(ResponseKind::Dashboard)).expect("dashboard");
        assert_eq!(view.created_at_ms, 1_790_848_800_000);
        assert_eq!(view.updated_at_ms, 1_790_935_200_500);

        let mut summary = kit(ResponseKind::Dashboard);
        summary.created_at = None;
        summary.updated_at = Some("not a date".to_string());
        let view = kit_to_view(&summary).expect("dashboard");
        assert_eq!(view.created_at_ms, 0);
        assert_eq!(view.updated_at_ms, 0);
    }

    #[test]
    fn negative_sizes_and_counts_are_clamped() {
        let mut summary = kit(ResponseKind::Dashboard);
        summary.total_payload_bytes = -1;
        summary.page_count = -4;
        summary.dataset_count = i64::from(u32::MAX) + 10;
        let view = kit_to_view(&summary).expect("dashboard");
        assert_eq!(view.byte_size, 0);
        assert_eq!(view.page_count, 0);
        assert_eq!(view.dataset_count, u32::MAX);
    }

    #[test]
    fn a_source_conversation_is_carried_over() {
        let mut summary = kit(ResponseKind::Dashboard);
        summary.source_conversation_id = Some("conv-9".to_string());
        let view = kit_to_view(&summary).expect("dashboard");
        assert_eq!(
            view.source_conversation_id,
            Some(ConversationId::from("conv-9".to_string()))
        );
    }

    #[test]
    fn a_title_is_trimmed_and_bounded_at_200_characters() {
        assert_eq!(clean_title("  Q3 pipeline  ").unwrap(), "Q3 pipeline");
        let at_limit = "a".repeat(200);
        assert_eq!(clean_title(&at_limit).unwrap(), at_limit);
        let over_limit = "a".repeat(201);
        assert_eq!(
            clean_title(&over_limit).unwrap_err().code,
            ReportErrorCode::InvalidTitle
        );
    }

    #[test]
    fn a_title_counts_characters_not_bytes() {
        let two_hundred_accented = "é".repeat(200);
        assert_eq!(
            clean_title(&two_hundred_accented).unwrap(),
            two_hundred_accented
        );
    }

    #[test]
    fn an_empty_whitespace_or_multiline_title_is_refused() {
        for title in [
            "",
            "   ",
            "\t\n",
            "Q3\npipeline",
            "Q3\u{0}pipeline",
            "Q3\u{2028}pipeline",
            "\u{200B}",
            "\u{202E}Q3",
            "Q3\u{FEFF}",
        ] {
            assert_eq!(
                clean_title(title).unwrap_err().code,
                ReportErrorCode::InvalidTitle,
                "title {title:?}"
            );
        }
    }

    #[test]
    fn names_in_other_scripts_and_emoji_sequences_are_accepted() {
        // ZWNJ in Persian, ZWJ in an emoji sequence, CJK, and an ordinary name.
        for title in [
            "\u{0645}\u{06CC}\u{200C}\u{062E}\u{0648}\u{0627}\u{0647}\u{0645}",
            "Team \u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
            "\u{58F2}\u{4E0A}\u{30EC}\u{30DD}\u{30FC}\u{30C8}",
            "Q3 pipeline",
        ] {
            assert_eq!(clean_title(title).as_deref(), Ok(title), "title {title:?}");
        }
    }

    #[test]
    fn a_kit_with_no_usable_id_is_not_listed() {
        let mut kit = kit(ResponseKind::Dashboard);
        kit.kit_id = String::new();
        assert!(kit_to_view(&kit).is_none());
        kit.kit_id = "kit-1".to_string();
        assert!(kit_to_view(&kit).is_none());
    }

    #[test]
    fn a_description_is_trimmed_and_empty_means_none() {
        assert_eq!(clean_description(None).unwrap(), None);
        assert_eq!(clean_description(Some("   ")).unwrap(), None);
        assert_eq!(
            clean_description(Some(" Weekly ")).unwrap(),
            Some("Weekly".to_string())
        );
    }

    #[test]
    fn a_description_over_4000_characters_is_refused_not_cut() {
        let long = "b".repeat(4001);
        assert_eq!(
            clean_description(Some(&long)).unwrap_err().code,
            ReportErrorCode::InvalidTitle
        );
        let exact = "c".repeat(4000);
        assert_eq!(clean_description(Some(&exact)).unwrap(), Some(exact));
    }

    #[test]
    fn a_kit_id_is_a_canonical_uuid() {
        assert!(valid_kit_id("6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01"));
        assert!(valid_kit_id("6B0F5D6E-0B7A-4C1E-9A53-2F5D9D3C1A01"));
        for bad in [
            "",
            "kit-1",
            ".",
            "..",
            "a/../b",
            "%2e%2e",
            "6b0f5d6e0b7a4c1e9a532f5d9d3c1a01",
            "urn:uuid:6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01",
            "{6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01}",
            "6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01\n",
        ] {
            assert!(!valid_kit_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_page_name_is_non_empty_bounded_and_printable() {
        assert!(valid_page_name("index.html"));
        assert!(valid_page_name("reports/detail page.html"));
        assert!(!valid_page_name(""));
        assert!(!valid_page_name(&"p".repeat(257)));
        assert!(valid_page_name(&"p".repeat(256)));
        assert!(!valid_page_name("a\nb"));
        assert!(!valid_page_name("a\u{0}b"));
    }

    #[test]
    fn kit_errors_map_by_the_table() {
        let cases = [
            (DaKitsApiError::NotFound, ReportErrorCode::NotFound),
            (DaKitsApiError::Forbidden, ReportErrorCode::Forbidden),
            (
                DaKitsApiError::Validation,
                ReportErrorCode::StorageUnavailable,
            ),
            (
                DaKitsApiError::Unauthorized,
                ReportErrorCode::StorageUnavailable,
            ),
            (
                DaKitsApiError::Conflict,
                ReportErrorCode::StorageUnavailable,
            ),
            (
                DaKitsApiError::PayloadTooLarge,
                ReportErrorCode::StorageUnavailable,
            ),
            (
                DaKitsApiError::RequestFailed,
                ReportErrorCode::StorageUnavailable,
            ),
            (
                DaKitsApiError::InvalidResponse,
                ReportErrorCode::StorageUnavailable,
            ),
        ];
        for (err, code) in cases {
            assert_eq!(map_kits_error(err).code, code, "{err:?}");
        }
    }
}
