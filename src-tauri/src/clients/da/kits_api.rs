//! DA client for dashboard report kits (`/api/v1/user/report-kits`).
//!
//! Replaces the chat reports client (design.md D1, D4, D5). DA stores a kit's
//! pages and dataset payloads verbatim and re-runs the stored SQL on refresh
//! under the caller's identity.

use std::time::Duration;

use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use thiserror::Error;

use super::generated::{
    KitPageResponse, KitRefreshCreate, KitRefreshResponse, KitSummaryResponse,
    PageKitSummaryResponse,
};
use super::mapping::{
    build_hardened_client, build_long_request_client, DaBoundaryError, DaRoute, TenantBaseUrl,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Error)]
pub(crate) enum DaKitsApiError {
    #[error("not signed in")]
    Unauthorized,
    #[error("not allowed to use this report")]
    Forbidden,
    #[error("report not found")]
    NotFound,
    #[error("report conflicts with existing record")]
    Conflict,
    #[error("report content exceeds size limit")]
    PayloadTooLarge,
    #[error("report payload is invalid")]
    Validation,
    #[error("tenant report kits API request failed")]
    RequestFailed,
    #[error("tenant report kits API returned an unexpected response")]
    InvalidResponse,
}

impl From<DaBoundaryError> for DaKitsApiError {
    fn from(_: DaBoundaryError) -> Self {
        Self::RequestFailed
    }
}

/// One PATCH, in the terms the data plane reads it.
///
/// DA resolves a patch by `model_fields_set`: an absent field is left alone, and
/// a field sent as `null` is cleared. The generated [`KitUpdate`] cannot say
/// the difference — `#[serde_with::skip_serializing_none]` drops a `None` before
/// it reaches the wire — so clearing a description was silently a no-op.
/// `Option<Option<String>>` says both: the outer `None` omits the field, and
/// `Some(None)` sends `null`.
#[derive(Default)]
pub(crate) struct KitPatchFields {
    pub(crate) name: Option<String>,
    pub(crate) description: Option<Option<String>>,
    pub(crate) pinned: Option<bool>,
    pub(crate) hidden: Option<bool>,
    /// `user` (private) or `global` (the organization). DA reads anything else
    /// as a validation failure, so only those two are ever built.
    pub(crate) scope: Option<&'static str>,
}

impl KitPatchFields {
    /// Builds the body field by field, so an omitted field is genuinely absent
    /// from the JSON rather than present and null.
    fn into_body(self) -> serde_json::Value {
        let mut body = serde_json::Map::new();
        if let Some(name) = self.name {
            body.insert("name".to_string(), serde_json::Value::String(name));
        }
        if let Some(description) = self.description {
            body.insert(
                "description".to_string(),
                match description {
                    Some(text) => serde_json::Value::String(text),
                    None => serde_json::Value::Null,
                },
            );
        }
        if let Some(pinned) = self.pinned {
            body.insert("pinned".to_string(), serde_json::Value::Bool(pinned));
        }
        if let Some(hidden) = self.hidden {
            body.insert("hidden".to_string(), serde_json::Value::Bool(hidden));
        }
        if let Some(scope) = self.scope {
            body.insert(
                "scope".to_string(),
                serde_json::Value::String(scope.to_string()),
            );
        }
        serde_json::Value::Object(body)
    }
}

/// Maximum pages walked when listing kits.
const MAX_LIST_PAGES: usize = 200;
/// The whole library walk may take at most this long, however many pages it has.
const LIST_BUDGET: Duration = Duration::from_secs(120);
/// Passes over the library when a concurrent write makes one miscount.
const MAX_WALK_ATTEMPTS: usize = 3;
/// Page size requested when listing kits.
const PAGE_SIZE: u32 = 100;
/// A refresh re-runs every dataset's SQL, which can outlast the ordinary 30 s
/// read and 45 s total timeouts (DA's own refresh limit is 120 s), so refresh
/// uses its own client bounded by this instead.
const REFRESH_TIMEOUT: Duration = Duration::from_secs(180);
/// A filled workbook or deck can be large, and its body is read after the
/// request is answered, so the download is bounded by this total instead of the
/// ordinary 45 s.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

pub(crate) struct DaKitsSession {
    base: TenantBaseUrl,
    client: reqwest::Client,
    bearer: String,
    /// Whether a download may follow DA's redirect to a plain `http` address.
    /// Only the loopback test server needs it, so only `for_test` sets it.
    plain_http_download: bool,
}

/// An id placed in a path must be a real segment. The URL builder drops the
/// literal segments `.` and `..` (and an empty one), which would send the request
/// to a different route, so those never leave this module.
fn require_segment(id: &str) -> Result<&str, DaKitsApiError> {
    if id.is_empty() || id == "." || id == ".." {
        return Err(DaKitsApiError::NotFound);
    }
    Ok(id)
}

impl DaKitsSession {
    pub(crate) fn new(da_origin: &str, bearer: impl Into<String>) -> Result<Self, DaKitsApiError> {
        let base = TenantBaseUrl::parse(da_origin)?;
        let client = build_hardened_client()?;
        Ok(Self {
            base,
            client,
            bearer: bearer.into(),
            plain_http_download: false,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        loopback_origin: &str,
        bearer: impl Into<String>,
    ) -> Result<Self, DaKitsApiError> {
        let base = TenantBaseUrl::parse_loopback(loopback_origin)?;
        let client = build_hardened_client()?;
        Ok(Self {
            base,
            client,
            bearer: bearer.into(),
            plain_http_download: true,
        })
    }

    /// Lists the caller's visible kits, draining pages by offset
    /// (`GET /api/v1/user/report-kits`). Kits the caller hid are included and
    /// flagged: hiding is a preference of the caller's own list, so the surface
    /// decides whether to show them.
    pub(crate) async fn list_kits(&self) -> Result<Vec<KitSummaryResponse>, DaKitsApiError> {
        // Each request has its own timeout, but up to 200 of them in a row do not
        // add up to a bounded read: this is the whole walk's budget.
        tokio::time::timeout(LIST_BUDGET, self.walk_library())
            .await
            .map_err(|_| DaKitsApiError::RequestFailed)?
    }

    /// Reads the library, once more when a write moved a kit between two pages.
    async fn walk_library(&self) -> Result<Vec<KitSummaryResponse>, DaKitsApiError> {
        for _ in 0..MAX_WALK_ATTEMPTS {
            if let Some(kits) = self.walk_pages().await? {
                return Ok(kits);
            }
        }
        // Still changing under every attempt: a library that cannot be read whole
        // is an error, never a shorter list.
        Err(DaKitsApiError::RequestFailed)
    }

    /// One request for the page at `offset`.
    async fn page_at(&self, offset: i64) -> Result<PageKitSummaryResponse, DaKitsApiError> {
        let mut url = self.base.url_for(DaRoute::ReportKits);
        url.query_pairs_mut()
            .append_pair("limit", &PAGE_SIZE.to_string())
            .append_pair("offset", &offset.to_string())
            .append_pair("include_hidden", "true");
        parse(send(self.client.get(url).bearer_auth(&self.bearer)).await?).await
    }

    /// One pass over every page. `None` when DA's total changed between pages, the
    /// kits read did not add up to it, or the first page moved while the rest was
    /// being read.
    ///
    /// DA pages by offset over a sort that a rename or a pin changes, so a kit
    /// written between two requests can move behind the cursor and be missed (or
    /// be read twice). The count is the proof that each kit was read exactly once
    /// (`total` of them), except for one case it cannot see: a delete and a create
    /// inside the same walk leave the total, and so the count, unchanged. A new or
    /// changed kit sorts first, so the first page is read again at the end, and a
    /// pass that spanned several pages is accepted only when it still matches.
    async fn walk_pages(&self) -> Result<Option<Vec<KitSummaryResponse>>, DaKitsApiError> {
        let mut kits: Vec<KitSummaryResponse> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        let mut offset: i64 = 0;
        let mut first_total: Option<i64> = None;
        let mut first_page_ids: Vec<String> = Vec::new();
        let mut pages_read = 0usize;

        for _ in 0..MAX_LIST_PAGES {
            let page = self.page_at(offset).await?;
            pages_read += 1;
            if pages_read == 1 {
                first_page_ids = page.items.iter().map(|kit| kit.kit_id.clone()).collect();
            }

            let received = page.items.len();
            let total = page.total;
            // A total that moved between pages means a kit was added or deleted
            // mid-walk, and a count that happens to match afterwards would hide it.
            if *first_total.get_or_insert(total) != total {
                return Ok(None);
            }
            for kit in page.items {
                if seen.insert(kit.kit_id.clone()) {
                    kits.push(kit);
                }
            }
            offset += received as i64;
            if offset >= total {
                if kits.len() as i64 != total {
                    return Ok(None);
                }
                // A single page is one answer and cannot have moved under itself.
                if pages_read > 1 {
                    let again = self.page_at(0).await?;
                    let unchanged = again.total == total
                        && again
                            .items
                            .iter()
                            .map(|kit| kit.kit_id.as_str())
                            .eq(first_page_ids.iter().map(String::as_str));
                    if !unchanged {
                        return Ok(None);
                    }
                }
                return Ok(Some(kits));
            }
            // An empty page while DA still says more exist cannot be told from a
            // library that was cut short, and a short library reads as deleted
            // reports: fail the read rather than return it.
            if received == 0 {
                return Err(DaKitsApiError::InvalidResponse);
            }
        }

        // The cap ran out with kits still unread. A silently shortened library
        // would read as reports that were deleted, so the read fails instead.
        Err(DaKitsApiError::RequestFailed)
    }

    /// Fetches one kit the caller can see (`GET /api/v1/user/report-kits/{kit_id}`),
    /// hidden or not: hiding is a preference, not access control.
    pub(crate) async fn get_kit(&self, kit_id: &str) -> Result<KitSummaryResponse, DaKitsApiError> {
        let url = self
            .base
            .scoped_url_for(DaRoute::ReportKits, require_segment(kit_id)?)?;
        parse(send(self.client.get(url).bearer_auth(&self.bearer)).await?).await
    }

    /// Fetches the assembled HTML for one page of a kit
    /// (`GET /api/v1/user/report-kits/{kit_id}/pages?page=`). `page` is omitted
    /// from the query when `None`, so DA picks the default page.
    pub(crate) async fn get_page(
        &self,
        kit_id: &str,
        page: Option<&str>,
    ) -> Result<KitPageResponse, DaKitsApiError> {
        let mut url = self
            .base
            .nested_url_for(DaRoute::ReportKits, &[require_segment(kit_id)?, "pages"])?;
        if let Some(page) = page {
            url.query_pairs_mut().append_pair("page", page);
        }
        parse(send(self.client.get(url).bearer_auth(&self.bearer)).await?).await
    }

    /// Fetches a kit's stored thumbnail
    /// (`GET /api/v1/user/report-kits/{kit_id}/thumbnail`). A 404 means the kit
    /// holds none, or is not visible to the caller.
    ///
    /// The answer must declare PNG or WebP, fit DA's own bound, and start with
    /// that format's signature. Anything else is not an image DA stored, and is
    /// refused before it reaches the renderer.
    pub(crate) async fn get_thumbnail(&self, kit_id: &str) -> Result<KitThumbnail, DaKitsApiError> {
        let url = self.base.nested_url_for(
            DaRoute::ReportKits,
            &[require_segment(kit_id)?, "thumbnail"],
        )?;
        let mut response = send(self.client.get(url).bearer_auth(&self.bearer)).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(error_for_status(status));
        }
        let declared = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(|value| value.trim().to_ascii_lowercase());
        let format = match declared.as_deref() {
            Some("image/png") => ThumbnailFormat::Png,
            Some("image/webp") => ThumbnailFormat::Webp,
            _ => return Err(DaKitsApiError::InvalidResponse),
        };
        if response
            .content_length()
            .is_some_and(|length| length > MAX_THUMBNAIL_BYTES as u64)
        {
            return Err(DaKitsApiError::InvalidResponse);
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| DaKitsApiError::InvalidResponse)?
        {
            if bytes.len() + chunk.len() > MAX_THUMBNAIL_BYTES {
                return Err(DaKitsApiError::InvalidResponse);
            }
            bytes.extend_from_slice(&chunk);
        }
        if !format.signs(&bytes) {
            return Err(DaKitsApiError::InvalidResponse);
        }
        Ok(KitThumbnail { format, bytes })
    }

    /// Stores a PNG as the thumbnail of a kit the caller owns
    /// (`PUT /api/v1/user/report-kits/{kit_id}/thumbnail`), replacing any image
    /// it held. DA answers 404 for a kit the caller does not own, and checks the
    /// image again itself.
    pub(crate) async fn put_thumbnail(
        &self,
        kit_id: &str,
        png: Vec<u8>,
    ) -> Result<KitSummaryResponse, DaKitsApiError> {
        let url = self.base.nested_url_for(
            DaRoute::ReportKits,
            &[require_segment(kit_id)?, "thumbnail"],
        )?;
        let request = self
            .client
            .put(url)
            .bearer_auth(&self.bearer)
            .header(reqwest::header::CONTENT_TYPE, "image/png")
            .body(png);
        parse(send(request).await?).await
    }

    /// Re-runs every dataset of a kit (`POST /api/v1/user/report-kits/{kit_id}/refresh`).
    /// The body is an empty object, which DA reads as "refresh all datasets".
    pub(crate) async fn refresh_kit(
        &self,
        kit_id: &str,
    ) -> Result<KitRefreshResponse, DaKitsApiError> {
        let url = self
            .base
            .nested_url_for(DaRoute::ReportKits, &[require_segment(kit_id)?, "refresh"])?;
        let body = KitRefreshCreate {
            datasets: None,
            params: None,
        };
        // Built here, not with the session: only a refresh needs it, and every
        // other operation would pay for a second client it never uses.
        let request = build_long_request_client(REFRESH_TIMEOUT)?
            .post(url)
            .bearer_auth(&self.bearer)
            .json(&body);
        parse(send(request).await?).await
    }

    /// Opens a workbook's or deck's filled file for reading
    /// (`GET /api/v1/user/report-kits/{kit_id}/file`).
    ///
    /// DA answers with a redirect to a short-lived signed URL, on object storage
    /// or on its own signed objects route. The signed URL authorizes itself and may
    /// belong to another host, so the bearer is sent to DA only and never to the
    /// target. One hop is followed and only to `https` (loopback `http` in tests);
    /// a target with credentials, or a second redirect, is refused.
    pub(crate) async fn open_file(&self, kit_id: &str) -> Result<KitFileDownload, DaKitsApiError> {
        let url = self
            .base
            .nested_url_for(DaRoute::ReportKits, &[require_segment(kit_id)?, "file"])?;
        let asked = send(self.client.get(url).bearer_auth(&self.bearer)).await?;
        let status = asked.status();
        if !status.is_redirection() {
            // A 2xx here is not what DA documents, and nothing but the redirect's
            // target is trusted to carry a file.
            return Err(if status.is_success() {
                DaKitsApiError::InvalidResponse
            } else {
                error_for_status(status)
            });
        }
        let target = asked
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|text| reqwest::Url::parse(text).ok())
            .ok_or(DaKitsApiError::InvalidResponse)?;
        let scheme_ok =
            target.scheme() == "https" || (self.plain_http_download && target.scheme() == "http");
        if !scheme_ok || !target.username().is_empty() || target.password().is_some() {
            return Err(DaKitsApiError::InvalidResponse);
        }
        let response = send(build_long_request_client(DOWNLOAD_TIMEOUT)?.get(target)).await?;
        let status = response.status();
        if !status.is_success() {
            return Err(error_for_status(status));
        }
        Ok(KitFileDownload { response })
    }

    /// Partially updates kit metadata (`PATCH /api/v1/user/report-kits/{kit_id}`).
    pub(crate) async fn patch_kit(
        &self,
        kit_id: &str,
        fields: KitPatchFields,
    ) -> Result<KitSummaryResponse, DaKitsApiError> {
        let url = self
            .base
            .scoped_url_for(DaRoute::ReportKits, require_segment(kit_id)?)?;
        let request = self
            .client
            .patch(url)
            .bearer_auth(&self.bearer)
            .json(&fields.into_body());
        parse(send(request).await?).await
    }

    /// Deletes a kit (`DELETE /api/v1/user/report-kits/{kit_id}`).
    pub(crate) async fn delete_kit(&self, kit_id: &str) -> Result<(), DaKitsApiError> {
        let url = self
            .base
            .scoped_url_for(DaRoute::ReportKits, require_segment(kit_id)?)?;
        let response = send(self.client.delete(url).bearer_auth(&self.bearer)).await?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        Err(error_for_status(status))
    }
}

/// The most a stored thumbnail may hold. DA refuses an upload past 200 KiB.
const MAX_THUMBNAIL_BYTES: usize = 200 * 1024;

/// The two image formats DA stores a thumbnail in.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ThumbnailFormat {
    Png,
    Webp,
}

impl ThumbnailFormat {
    /// Whether `bytes` start with this format's signature: PNG's eight-byte
    /// header, or WebP's `RIFF` container naming `WEBP`.
    pub(crate) fn signs(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
            Self::Webp => {
                bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP"
            }
        }
    }
}

/// A kit's stored thumbnail, checked to be the image it says it is.
pub(crate) struct KitThumbnail {
    pub(crate) format: ThumbnailFormat,
    pub(crate) bytes: Vec<u8>,
}

/// A filled workbook's or deck's body, read a chunk at a time so a large file is
/// never held whole in memory.
pub(crate) struct KitFileDownload {
    response: reqwest::Response,
}

impl KitFileDownload {
    /// The length the target declared, when it did.
    pub(crate) fn declared_length(&self) -> Option<u64> {
        self.response.content_length()
    }

    /// The next piece of the body, or `None` at its end.
    pub(crate) async fn chunk(&mut self) -> Result<Option<bytes::Bytes>, DaKitsApiError> {
        self.response
            .chunk()
            .await
            .map_err(|_| DaKitsApiError::RequestFailed)
    }
}

async fn send(request: reqwest::RequestBuilder) -> Result<reqwest::Response, DaKitsApiError> {
    request
        .send()
        .await
        .map_err(|_| DaKitsApiError::RequestFailed)
}

/// The most a 2xx body may hold. The largest legitimate answer is one page of
/// HTML (at most 2 MiB to open, a little more once JSON-escaped); anything past
/// this is not a report, and reading it whole would only fill memory first.
const MAX_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Parses a 2xx body. Any other status maps to an error without reading the
/// body, so a response body never reaches an error value or a log.
async fn parse<T: DeserializeOwned>(mut response: reqwest::Response) -> Result<T, DaKitsApiError> {
    let status = response.status();
    if !status.is_success() {
        return Err(error_for_status(status));
    }
    // A declared length is refused up front; a body that does not declare one is
    // cut off as it arrives.
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY_BYTES as u64)
    {
        return Err(DaKitsApiError::InvalidResponse);
    }
    let mut body: Vec<u8> = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| DaKitsApiError::InvalidResponse)?
    {
        if body.len() + chunk.len() > MAX_BODY_BYTES {
            return Err(DaKitsApiError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| DaKitsApiError::InvalidResponse)
}

/// Maps a non-2xx status. 401 and 403 stay distinct: a 401 means the bearer is
/// no longer valid, while a 403 means the signed-in user lacks the grant.
fn error_for_status(status: StatusCode) -> DaKitsApiError {
    match status.as_u16() {
        401 => DaKitsApiError::Unauthorized,
        403 => DaKitsApiError::Forbidden,
        404 => DaKitsApiError::NotFound,
        409 => DaKitsApiError::Conflict,
        413 => DaKitsApiError::PayloadTooLarge,
        422 => DaKitsApiError::Validation,
        _ => DaKitsApiError::RequestFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{
        body_json, header, method, path, query_param, query_param_is_missing,
    };
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// The error from a call whose success type has no `Debug`, which the
    /// generated kit DTOs do not derive.
    fn err_of<T>(result: Result<T, DaKitsApiError>) -> DaKitsApiError {
        match result {
            Ok(_) => panic!("expected an error"),
            Err(err) => err,
        }
    }

    fn kit_json(kit_id: &str) -> serde_json::Value {
        serde_json::json!({
            "kit_id": kit_id,
            "name": "Q3 pipeline",
            "kind": "dashboard",
            "scope": "user",
            "owner_user_id": "user-1",
            "owner_display_name": null,
            "is_owner": true,
            "refreshable": true,
            "can_refresh": true,
            "can_manage": true,
            "can_edit": false,
            "can_delete": true,
            "source_artifact_id": null,
            "source_conversation_id": null,
            "page_count": 1,
            "dataset_count": 1,
            "total_payload_bytes": 0,
            "created_at": null,
            "updated_at": null,
            "description": null,
            "pinned": false,
            "hidden": false
        })
    }

    fn page_json(items: Vec<serde_json::Value>, total: i64, offset: i64) -> serde_json::Value {
        serde_json::json!({
            "items": items,
            "total": total,
            "limit": 100,
            "offset": offset
        })
    }

    #[tokio::test]
    async fn list_kits_drains_two_pages() {
        let server = MockServer::start().await;
        let first: Vec<serde_json::Value> =
            (0..100).map(|i| kit_json(&format!("kit-{i}"))).collect();

        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(query_param("limit", "100"))
            .and(query_param("offset", "0"))
            .and(query_param("include_hidden", "true"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(first, 101, 0)))
            // Read again at the end of a multi-page pass, to confirm it held still.
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(query_param("limit", "100"))
            .and(query_param("offset", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(
                vec![kit_json("kit-100")],
                101,
                100,
            )))
            .expect(1)
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let kits = session.list_kits().await.unwrap();
        assert_eq!(kits.len(), 101);
        assert_eq!(kits[100].kit_id, "kit-100");
    }

    #[tokio::test]
    async fn list_kits_fails_on_an_empty_page_while_more_are_promised() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(
                vec![kit_json("kit-1")],
                10,
                0,
            )))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(query_param("offset", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(vec![], 10, 1)))
            .expect(1)
            .mount(&server)
            .await;

        // A library that stopped short reads as deleted reports, so it is an error.
        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(
            err_of(session.list_kits().await),
            DaKitsApiError::InvalidResponse
        );
    }

    #[tokio::test]
    async fn list_kits_reads_again_when_the_library_shrank_between_pages() {
        let server = MockServer::start().await;
        // First pass: a kit was deleted meanwhile, so the second page is empty and
        // DA's total fell from 2 to 1.
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(
                vec![kit_json("kit-1")],
                2,
                0,
            )))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(query_param("offset", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(vec![], 1, 1)))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Second pass: one kit, and DA says so.
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(
                vec![kit_json("kit-1")],
                1,
                0,
            )))
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(session.list_kits().await.unwrap().len(), 1);
    }

    fn hundred_kits() -> Vec<serde_json::Value> {
        (0..100).map(|i| kit_json(&format!("kit-{i}"))).collect()
    }

    /// Mounts one page of a 101-kit library for the given offset.
    async fn mount_page(
        server: &MockServer,
        offset: i64,
        items: Vec<serde_json::Value>,
        times: Option<u64>,
    ) {
        let mock = Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(query_param("offset", offset.to_string()))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(items, 101, offset)));
        match times {
            Some(times) => mock.up_to_n_times(times).mount(server).await,
            None => mock.mount(server).await,
        }
    }

    #[tokio::test]
    async fn list_kits_reads_again_when_a_kit_moved_between_pages() {
        let server = MockServer::start().await;
        // First pass: the library shifted by one, so the last kit of page one
        // comes back on page two and `kit-100` is never read. The count is short.
        mount_page(&server, 0, hundred_kits(), Some(1)).await;
        mount_page(&server, 100, vec![kit_json("kit-99")], Some(1)).await;
        // Second pass: the library held still.
        mount_page(&server, 0, hundred_kits(), None).await;
        mount_page(&server, 100, vec![kit_json("kit-100")], None).await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let kits = session.list_kits().await.unwrap();
        assert_eq!(kits.len(), 101);
        assert_eq!(kits[100].kit_id, "kit-100");
    }

    #[tokio::test]
    async fn list_kits_reads_again_when_a_kit_was_deleted_between_pages() {
        let server = MockServer::start().await;
        // First pass: a kit already read is deleted, so page two reports a total of
        // 100 and no kits. 100 kits were read, which equals that total, but the
        // list holds the deleted kit and misses a live one.
        mount_page(&server, 0, hundred_kits(), Some(1)).await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(query_param("offset", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(vec![], 100, 100)))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Second pass: a library of 100 that held still.
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .and(query_param("offset", "0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(
                (0..100).map(|i| kit_json(&format!("live-{i}"))).collect(),
                100,
                0,
            )))
            .mount(&server)
            .await;

        // The answer is the second pass, not the first pass that merely counted right.
        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let kits = session.list_kits().await.unwrap();
        assert_eq!(kits.len(), 100);
        assert_eq!(kits[0].kit_id, "live-0");
    }

    #[tokio::test]
    async fn list_kits_reads_again_when_a_delete_and_a_create_landed_in_one_walk() {
        let server = MockServer::start().await;
        let ids = |prefix: &str, from: i64, to: i64| -> Vec<serde_json::Value> {
            (from..to)
                .map(|i| kit_json(&format!("{prefix}-{i}")))
                .collect()
        };
        let page = |items: Vec<serde_json::Value>, offset: i64| {
            ResponseTemplate::new(200).set_body_json(page_json(items, 150, offset))
        };
        let mount = |offset: &'static str, template: ResponseTemplate, times: Option<u64>| {
            let mock = Mock::given(method("GET"))
                .and(path("/api/v1/user/report-kits"))
                .and(query_param("offset", offset))
                .respond_with(template);
            (mock, times)
        };
        // First pass: both pages read a library of 150 whose total never moved, but
        // between the pages one kit was deleted and another created on top. The
        // count still adds up; only the first page, read again, shows it.
        for (mock, times) in [
            mount("0", page(ids("old", 0, 100), 0), Some(1)),
            mount("100", page(ids("old", 100, 150), 100), Some(1)),
            // The anchor read: the library's top is now another kit.
            mount("0", page(ids("moved", 0, 100), 0), Some(1)),
            // Second pass: a library that held still.
            mount("0", page(ids("live", 0, 100), 0), None),
            mount("100", page(ids("live", 100, 150), 100), None),
        ] {
            match times {
                Some(times) => mock.up_to_n_times(times).mount(&server).await,
                None => mock.mount(&server).await,
            }
        }

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let kits = session.list_kits().await.unwrap();

        // The answer is the second pass, not the first pass that counted right.
        assert_eq!(kits.len(), 150);
        assert_eq!(kits[0].kit_id, "live-0");
        assert_eq!(kits[149].kit_id, "live-149");
    }

    #[tokio::test]
    async fn a_response_body_past_the_cap_is_refused() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path(format!(
                "/api/v1/user/report-kits/{}/pages",
                "6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01"
            )))
            // A well-formed page whose html alone is past the cap, so only the cap
            // can refuse it: blank bytes would fail to parse either way.
            .respond_with(ResponseTemplate::new(200).set_body_string(format!(
                r#"{{"kit_id":"6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01","kit_name":"x","page":"index.html","pages":["index.html"],"html":"{}"}}"#,
                "a".repeat(MAX_BODY_BYTES)
            )))
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(
            err_of(
                session
                    .get_page("6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01", None)
                    .await
            ),
            DaKitsApiError::InvalidResponse
        );
    }

    #[tokio::test]
    async fn a_server_error_a_throttle_and_a_dead_connection_are_a_failed_request() {
        let id = "6b0f5d6e-0b7a-4c1e-9a53-2f5d9d3c1a01";
        for status in [429u16, 500, 502, 503] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(format!("/api/v1/user/report-kits/{id}/pages")))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
            assert_eq!(
                err_of(session.get_page(id, None).await),
                DaKitsApiError::RequestFailed,
                "status {status}"
            );
        }
        // A port that was just free: nothing is listening, and it is refused at once.
        let free_port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let session =
            DaKitsSession::for_test(&format!("http://127.0.0.1:{free_port}"), "test-token")
                .unwrap();
        assert_eq!(
            err_of(session.get_page(id, None).await),
            DaKitsApiError::RequestFailed
        );
    }

    #[tokio::test]
    async fn list_kits_fails_when_a_page_holds_more_kits_than_the_total() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(
                vec![kit_json("kit-1"), kit_json("kit-2")],
                1,
                0,
            )))
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(
            err_of(session.list_kits().await),
            DaKitsApiError::RequestFailed
        );
    }

    #[tokio::test]
    async fn list_kits_fails_when_every_pass_misses_a_kit() {
        let server = MockServer::start().await;
        mount_page(&server, 0, hundred_kits(), None).await;
        mount_page(&server, 100, vec![kit_json("kit-99")], None).await;

        // A library that never adds up must not be returned as if it did.
        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(
            err_of(session.list_kits().await),
            DaKitsApiError::RequestFailed
        );
    }

    #[tokio::test]
    async fn an_id_that_is_a_dot_segment_never_leaves_the_client() {
        let server = MockServer::start().await;
        Mock::given(wiremock::matchers::any())
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        for id in ["", ".", ".."] {
            assert_eq!(
                err_of(session.delete_kit(id).await),
                DaKitsApiError::NotFound,
                "{id:?}"
            );
            assert_eq!(
                err_of(session.refresh_kit(id).await),
                DaKitsApiError::NotFound,
                "{id:?}"
            );
            assert_eq!(
                err_of(session.get_page(id, None).await),
                DaKitsApiError::NotFound,
                "{id:?}"
            );
            assert_eq!(
                err_of(session.patch_kit(id, KitPatchFields::default()).await),
                DaKitsApiError::NotFound,
                "{id:?}"
            );
        }
    }

    /// A `%` in an id is sent as `%25`, so `%2e%2e` is text and never a dot segment.
    #[tokio::test]
    async fn an_encoded_dot_segment_is_only_text() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/user/report-kits/%252e%252e"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        session.delete_kit("%2e%2e").await.unwrap();
    }

    #[tokio::test]
    async fn list_kits_reads_a_library_of_250_across_three_pages() {
        let server = MockServer::start().await;
        for (offset, count) in [(0, 100), (100, 100), (200, 50)] {
            let items: Vec<serde_json::Value> = (0..count)
                .map(|i| kit_json(&format!("kit-{}", offset + i)))
                .collect();
            Mock::given(method("GET"))
                .and(path("/api/v1/user/report-kits"))
                .and(query_param("offset", offset.to_string()))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(page_json(items, 250, offset)),
                )
                // The first page is read again at the end, to confirm it held still.
                .expect(if offset == 0 { 2 } else { 1 })
                .mount(&server)
                .await;
        }

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let kits = session.list_kits().await.unwrap();
        assert_eq!(kits.len(), 250);
        assert_eq!(kits[249].kit_id, "kit-249");
    }

    #[tokio::test]
    async fn list_kits_fails_rather_than_return_a_library_cut_at_the_page_cap() {
        let server = MockServer::start().await;

        // Every page reports more rows than the cap can reach, so only the cap
        // can end the walk.
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_json(
                vec![kit_json("kit-x")],
                100_000,
                0,
            )))
            .expect(MAX_LIST_PAGES as u64)
            .mount(&server)
            .await;

        // A shortened library would read as deleted reports, so the read fails.
        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(
            err_of(session.list_kits().await),
            DaKitsApiError::RequestFailed
        );
    }

    #[tokio::test]
    async fn list_kits_maps_401_and_403_to_different_errors() {
        for (status, expected) in [
            (401, DaKitsApiError::Unauthorized),
            (403, DaKitsApiError::Forbidden),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/user/report-kits"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;

            let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
            assert_eq!(err_of(session.list_kits().await), expected);
        }
    }

    #[tokio::test]
    async fn get_page_sends_the_page_query_only_when_given() {
        let server = MockServer::start().await;
        let page_body = serde_json::json!({
            "kit_id": "kit-1",
            "kit_name": "Q3 pipeline",
            "kind": "dashboard",
            "page": "index.html",
            "pages": ["index.html", "detail.html"],
            "html": "<h1>Pipeline</h1>",
            "size_bytes": 17,
            "sha256": "abc",
            "datasets": null
        });

        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/kit-1/pages"))
            .and(query_param_is_missing("page"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_body.clone()))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/kit-1/pages"))
            .and(query_param("page", "detail.html"))
            .respond_with(ResponseTemplate::new(200).set_body_json(page_body))
            .expect(1)
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let default_page = session.get_page("kit-1", None).await.unwrap();
        assert_eq!(default_page.html, "<h1>Pipeline</h1>");
        let named = session
            .get_page("kit-1", Some("detail.html"))
            .await
            .unwrap();
        assert_eq!(named.kit_id, "kit-1");
    }

    #[tokio::test]
    async fn patch_kit_sends_null_to_clear_a_description_and_omits_absent_keys() {
        let server = MockServer::start().await;

        Mock::given(method("PATCH"))
            .and(path("/api/v1/user/report-kits/kit-1"))
            .and(header("Authorization", "Bearer test-token"))
            .and(body_json(serde_json::json!({ "description": null })))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit_json("kit-1")))
            .expect(1)
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let fields = KitPatchFields {
            description: Some(None),
            ..Default::default()
        };
        let patched = session.patch_kit("kit-1", fields).await.unwrap();
        assert_eq!(patched.kit_id, "kit-1");
    }

    #[tokio::test]
    async fn patch_kit_sends_only_the_fields_that_are_set() {
        let server = MockServer::start().await;

        Mock::given(method("PATCH"))
            .and(path("/api/v1/user/report-kits/kit-1"))
            .and(body_json(serde_json::json!({
                "name": "Renamed",
                "description": "Now described",
                "pinned": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit_json("kit-1")))
            .expect(1)
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let fields = KitPatchFields {
            name: Some("Renamed".to_string()),
            description: Some(Some("Now described".to_string())),
            pinned: Some(false),
            ..Default::default()
        };
        session.patch_kit("kit-1", fields).await.unwrap();
    }

    #[tokio::test]
    async fn delete_kit_accepts_204_and_maps_404() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/api/v1/user/report-kits/kit-1"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/user/report-kits/kit-gone"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        session.delete_kit("kit-1").await.unwrap();
        assert_eq!(
            session.delete_kit("kit-gone").await.unwrap_err(),
            DaKitsApiError::NotFound
        );
    }

    #[tokio::test]
    async fn refresh_kit_posts_an_empty_object_and_parses_outcomes() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/user/report-kits/kit-1/refresh"))
            .and(header("Authorization", "Bearer test-token"))
            .and(body_json(serde_json::json!({})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "kit_id": "kit-1",
                "kind": "dashboard",
                "outcomes": [
                    { "dataset_id": "sales", "ok": true, "rows": 12 },
                    { "dataset_id": "stock", "ok": false, "error": "timeout" }
                ],
                "kit": kit_json("kit-1")
            })))
            .expect(1)
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let refreshed = session.refresh_kit("kit-1").await.unwrap();
        assert_eq!(refreshed.kit.kit_id, "kit-1");
        assert_eq!(refreshed.outcomes.len(), 2);
        assert!(refreshed.outcomes[0].ok);
        assert!(!refreshed.outcomes[1].ok);
        assert_eq!(refreshed.outcomes[1].error.as_deref(), Some("timeout"));
    }

    #[tokio::test]
    async fn an_unparseable_success_body_is_an_invalid_response() {
        let server = MockServer::start().await;

        Mock::given(method("PATCH"))
            .and(path("/api/v1/user/report-kits/kit-1"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let fields = KitPatchFields {
            pinned: Some(true),
            ..Default::default()
        };
        assert_eq!(
            err_of(session.patch_kit("kit-1", fields).await),
            DaKitsApiError::InvalidResponse
        );
    }

    /// An id is one path segment. A `/` in it must be sent as `%2F` so it
    /// cannot start a new segment, and `..` inside a segment is only text.
    #[tokio::test]
    async fn an_id_with_path_syntax_stays_one_segment() {
        let server = MockServer::start().await;

        Mock::given(method("DELETE"))
            .and(path("/api/v1/user/report-kits/kit%2F..%2Fother"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        session.delete_kit("kit/../other").await.unwrap();
    }

    #[tokio::test]
    async fn patch_kit_sends_scope_and_hidden_each_alone() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/user/report-kits/kit-1"))
            .and(body_json(serde_json::json!({ "scope": "global" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit_json("kit-1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/user/report-kits/kit-1"))
            .and(body_json(serde_json::json!({ "hidden": true })))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit_json("kit-1")))
            .expect(1)
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        session
            .patch_kit(
                "kit-1",
                KitPatchFields {
                    scope: Some("global"),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        session
            .patch_kit(
                "kit-1",
                KitPatchFields {
                    hidden: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn get_kit_reads_one_kit_and_maps_a_missing_one() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/kit-1"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit_json("kit-1")))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/kit-gone"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1)
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(session.get_kit("kit-1").await.unwrap().kit_id, "kit-1");
        assert_eq!(
            err_of(session.get_kit("kit-gone").await),
            DaKitsApiError::NotFound
        );
    }

    /// DA answers `/file` with a redirect to a signed URL.
    fn redirect_to(location: &str) -> ResponseTemplate {
        ResponseTemplate::new(307).insert_header("Location", location)
    }

    async fn read_all(mut download: KitFileDownload) -> Vec<u8> {
        let mut body = Vec::new();
        while let Some(chunk) = download.chunk().await.unwrap() {
            body.extend_from_slice(&chunk);
        }
        body
    }

    #[tokio::test]
    async fn open_file_follows_the_redirect_and_never_sends_the_bearer_to_it() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/kit-1/file"))
            .and(header("Authorization", "Bearer test-token"))
            .respond_with(redirect_to(&format!("{}/signed/abc?sig=1", server.uri())))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/signed/abc"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PK-workbook".to_vec()))
            .expect(1)
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        let download = session.open_file("kit-1").await.unwrap();
        assert_eq!(download.declared_length(), Some(11));
        assert_eq!(read_all(download).await, b"PK-workbook");

        // The signed URL authorizes itself, and may be another host's: nothing
        // that identifies the caller goes with it.
        let received = server.received_requests().await.unwrap();
        let signed = received
            .iter()
            .find(|request| request.url.path() == "/signed/abc")
            .expect("the target was requested");
        assert!(signed.headers.get("authorization").is_none());
        assert_eq!(signed.url.query(), Some("sig=1"));
    }

    #[tokio::test]
    async fn open_file_refuses_a_target_that_is_not_https() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/kit-1/file"))
            .respond_with(redirect_to("http://files.example.test/signed/abc"))
            .mount(&server)
            .await;

        // Only the loopback test session may use plain http.
        let mut session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        session.plain_http_download = false;
        assert_eq!(
            err_of(session.open_file("kit-1").await),
            DaKitsApiError::InvalidResponse
        );
        // And the refused target was never requested.
        let received = server.received_requests().await.unwrap();
        assert_eq!(received.len(), 1);
    }

    #[tokio::test]
    async fn open_file_refuses_a_target_that_carries_credentials() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/kit-1/file"))
            .respond_with(redirect_to("http://user:secret@127.0.0.1:9/signed/abc"))
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(
            err_of(session.open_file("kit-1").await),
            DaKitsApiError::InvalidResponse
        );
    }

    #[tokio::test]
    async fn open_file_refuses_a_redirect_it_cannot_read_and_a_second_hop() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/no-location/file"))
            .respond_with(ResponseTemplate::new(307))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/relative/file"))
            .respond_with(redirect_to("/signed/abc"))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/two-hops/file"))
            .respond_with(redirect_to(&format!("{}/hop", server.uri())))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/hop"))
            .respond_with(redirect_to(&format!("{}/elsewhere", server.uri())))
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        for id in ["no-location", "relative"] {
            assert_eq!(
                err_of(session.open_file(id).await),
                DaKitsApiError::InvalidResponse,
                "{id}"
            );
        }
        assert_eq!(
            err_of(session.open_file("two-hops").await),
            DaKitsApiError::RequestFailed
        );
    }

    #[tokio::test]
    async fn open_file_takes_no_body_from_da_itself() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/kit-1/file"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"not via redirect".to_vec()))
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(
            err_of(session.open_file("kit-1").await),
            DaKitsApiError::InvalidResponse
        );
    }

    #[tokio::test]
    async fn open_file_maps_what_da_and_the_target_answer() {
        let server = MockServer::start().await;
        for (id, status) in [("gone", 404), ("denied", 403), ("no-storage", 501)] {
            Mock::given(method("GET"))
                .and(path(format!("/api/v1/user/report-kits/{id}/file")))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
        }
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/expired/file"))
            .respond_with(redirect_to(&format!("{}/signed/old", server.uri())))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/signed/old"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(
            err_of(session.open_file("gone").await),
            DaKitsApiError::NotFound
        );
        assert_eq!(
            err_of(session.open_file("denied").await),
            DaKitsApiError::Forbidden
        );
        assert_eq!(
            err_of(session.open_file("no-storage").await),
            DaKitsApiError::RequestFailed
        );
        assert_eq!(
            err_of(session.open_file("expired").await),
            DaKitsApiError::Forbidden
        );
    }

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nIHDR-rest";
    const WEBP: &[u8] = b"RIFF\x10\x00\x00\x00WEBPVP8 rest";

    async fn thumbnail_server(template: ResponseTemplate) -> MockServer {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/user/report-kits/kit-1/thumbnail"))
            .and(header("authorization", "Bearer test-token"))
            .respond_with(template)
            .mount(&server)
            .await;
        server
    }

    fn image(content_type: &str, body: &[u8]) -> ResponseTemplate {
        ResponseTemplate::new(200).set_body_raw(body.to_vec(), content_type)
    }

    #[tokio::test]
    async fn a_png_or_webp_thumbnail_is_read_with_its_format() {
        for (content_type, body, format) in [
            ("image/png", PNG, ThumbnailFormat::Png),
            ("image/webp; charset=binary", WEBP, ThumbnailFormat::Webp),
        ] {
            let server = thumbnail_server(image(content_type, body)).await;
            let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
            let thumbnail = session.get_thumbnail("kit-1").await.unwrap();
            assert_eq!(thumbnail.format, format);
            assert_eq!(thumbnail.bytes, body);
        }
    }

    #[tokio::test]
    async fn a_kit_with_no_thumbnail_is_not_found() {
        let server = thumbnail_server(ResponseTemplate::new(404)).await;
        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(
            err_of(session.get_thumbnail("kit-1").await),
            DaKitsApiError::NotFound
        );
    }

    #[tokio::test]
    async fn a_thumbnail_that_is_not_the_image_it_declares_is_refused() {
        let oversized = [PNG, &vec![0u8; MAX_THUMBNAIL_BYTES]].concat();
        for template in [
            // Another type, or none.
            image("image/gif", b"GIF89a"),
            image("text/html", b"<html>sign in</html>"),
            // The declared type with another format's bytes.
            image("image/png", WEBP),
            image("image/webp", PNG),
            // Past DA's own bound.
            image("image/png", &oversized),
        ] {
            let server = thumbnail_server(template).await;
            let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
            assert_eq!(
                err_of(session.get_thumbnail("kit-1").await),
                DaKitsApiError::InvalidResponse
            );
        }
    }

    #[tokio::test]
    async fn a_thumbnail_is_stored_as_a_png_body_and_an_unowned_kit_is_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/user/report-kits/kit-1/thumbnail"))
            .and(header("authorization", "Bearer test-token"))
            .and(header("content-type", "image/png"))
            .and(wiremock::matchers::body_bytes(PNG.to_vec()))
            .respond_with(ResponseTemplate::new(200).set_body_json(kit_json("kit-1")))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/user/report-kits/kit-2/thumbnail"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let session = DaKitsSession::for_test(&server.uri(), "test-token").unwrap();
        assert_eq!(
            session
                .put_thumbnail("kit-1", PNG.to_vec())
                .await
                .unwrap()
                .kit_id,
            "kit-1"
        );
        assert_eq!(
            err_of(session.put_thumbnail("kit-2", PNG.to_vec()).await),
            DaKitsApiError::NotFound
        );
    }
}
