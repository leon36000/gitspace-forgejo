use super::{
    MAX_WEBHOOK_BODY_BYTES, SUPPORTED_EVENTS, WEBHOOK_EVENT_CONTRACTS_V1,
    WEBHOOK_EVENT_CONTRACTS_V2, WEBHOOK_EVENT_CONTRACTS_V3, WEBHOOK_EVENT_CONTRACTS_V4,
    WEBHOOK_EVENT_CONTRACTS_V5, WEBHOOK_EVENT_CONTRACTS_V6, WEBHOOK_EVENT_CONTRACTS_V7,
    WEBHOOK_EVENT_CONTRACTS_V8, WEBHOOK_EVENT_CONTRACTS_V9, WebhookError, ingest_forgejo_delivery,
    ingest_forgejo_delivery_attested,
};
use crate::{AppendResult, EVENT_JOURNAL_LOCK_WAIT_MS, EventJournal, JournalError};
use ed25519_dalek::SigningKey;
use serde_json::{Value, json};

pub const MAX_HEADER_BYTES: usize = 16 * 1024;
pub const SERVER_BUSY_RETRY_AFTER_SECONDS: u64 = 5;
const JOURNAL_BUSY_RETRY_AFTER_SECONDS: u64 = EVENT_JOURNAL_LOCK_WAIT_MS.div_ceil(1_000);

#[derive(Debug, PartialEq, Eq)]
pub enum HttpRequestError {
    Incomplete,
    Malformed,
    HeaderTooLarge,
    BodyTooLarge,
}

#[derive(Debug, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub target: String,
    pub version: String,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Value,
    pub retry_after_seconds: Option<u64>,
}

impl HttpRequest {
    pub fn single_header(&self, name: &str) -> Result<&str, HttpRequestError> {
        let name = name.to_ascii_lowercase();
        let mut values = self
            .headers
            .iter()
            .filter(|(header, _)| header == &name)
            .map(|(_, value)| value.as_str());
        let value = values.next().ok_or(HttpRequestError::Malformed)?;
        if values.next().is_some() || value.is_empty() {
            return Err(HttpRequestError::Malformed);
        }
        Ok(value)
    }

    pub fn header_alias(&self, forgejo: &str, gitea: &str) -> Result<&str, HttpRequestError> {
        let forgejo_value = self.optional_single_header(forgejo)?;
        let gitea_value = self.optional_single_header(gitea)?;
        match (forgejo_value, gitea_value) {
            (Some(left), Some(right)) if left != right => Err(HttpRequestError::Malformed),
            (Some(value), _) | (_, Some(value)) if !value.is_empty() => Ok(value),
            _ => Err(HttpRequestError::Malformed),
        }
    }

    fn optional_single_header(&self, name: &str) -> Result<Option<&str>, HttpRequestError> {
        let name = name.to_ascii_lowercase();
        let mut values = self
            .headers
            .iter()
            .filter(|(header, _)| header == &name)
            .map(|(_, value)| value.as_str());
        let Some(value) = values.next() else {
            return Ok(None);
        };
        if values.next().is_some() {
            return Err(HttpRequestError::Malformed);
        }
        Ok(Some(value))
    }
}

pub fn parse_http_request(bytes: &[u8]) -> Result<HttpRequest, HttpRequestError> {
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return if bytes.len() > MAX_HEADER_BYTES {
            Err(HttpRequestError::HeaderTooLarge)
        } else {
            Err(HttpRequestError::Incomplete)
        };
    };
    let body_start = header_end + 4;
    if body_start > MAX_HEADER_BYTES {
        return Err(HttpRequestError::HeaderTooLarge);
    }
    let header_bytes = &bytes[..header_end];
    let header_text = std::str::from_utf8(header_bytes).map_err(|_| HttpRequestError::Malformed)?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or(HttpRequestError::Malformed)?;
    let parts = request_line.split(' ').collect::<Vec<_>>();
    if parts.len() != 3
        || parts.iter().any(|part| part.is_empty())
        || !matches!(parts[2], "HTTP/1.0" | "HTTP/1.1")
        || !parts[1].starts_with('/')
        || parts[1].contains('#')
    {
        return Err(HttpRequestError::Malformed);
    }

    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() || line.starts_with([' ', '\t']) {
            return Err(HttpRequestError::Malformed);
        }
        let (name, value) = line.split_once(':').ok_or(HttpRequestError::Malformed)?;
        if name.is_empty() || !name.bytes().all(is_header_name_byte) {
            return Err(HttpRequestError::Malformed);
        }
        let value = value.trim_matches([' ', '\t']);
        if !value
            .bytes()
            .all(|byte| byte == b'\t' || (0x20..0x7f).contains(&byte))
        {
            return Err(HttpRequestError::Malformed);
        }
        headers.push((name.to_ascii_lowercase(), value.to_owned()));
    }

    let host_values = headers
        .iter()
        .filter(|(name, _)| name == "host")
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    // HTTP/1.1 needs one Host field; duplicate Host fields are ambiguous for
    // any request version, and an empty value cannot identify an authority.
    if host_values.len() > 1
        || host_values
            .iter()
            .any(|value| !is_valid_host_authority(value))
        || (parts[2] == "HTTP/1.1" && host_values.is_empty())
    {
        return Err(HttpRequestError::Malformed);
    }
    if headers.iter().any(|(name, _)| name == "transfer-encoding") {
        return Err(HttpRequestError::Malformed);
    }
    let lengths = headers
        .iter()
        .filter(|(name, _)| name == "content-length")
        .map(|(_, value)| value.as_str())
        .collect::<Vec<_>>();
    if lengths.len() > 1 {
        return Err(HttpRequestError::Malformed);
    }
    // A request without Content-Length has no body. Since this parser receives
    // exactly one complete request buffer, any bytes after the header block in
    // that case are ambiguous and must be rejected rather than treated as a
    // body or a pipelined request.
    let content_length = match lengths.first() {
        None => 0,
        Some(value) if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) => {
            return Err(HttpRequestError::Malformed);
        }
        Some(value) => value
            .parse::<usize>()
            .map_err(|_| HttpRequestError::BodyTooLarge)?,
    };
    if content_length > MAX_WEBHOOK_BODY_BYTES {
        return Err(HttpRequestError::BodyTooLarge);
    }
    if lengths.is_empty() && bytes.len() != body_start {
        return Err(HttpRequestError::Malformed);
    }
    let expected_end = body_start
        .checked_add(content_length)
        .ok_or(HttpRequestError::BodyTooLarge)?;
    if bytes.len() < expected_end {
        return Err(HttpRequestError::Incomplete);
    }
    if bytes.len() != expected_end {
        return Err(HttpRequestError::Malformed);
    }

    Ok(HttpRequest {
        method: parts[0].to_owned(),
        target: parts[1].to_owned(),
        version: parts[2].to_owned(),
        headers,
        body: bytes[body_start..expected_end].to_vec(),
    })
}

pub fn handle_webhook_request(
    request: &HttpRequest,
    secret: &[u8],
    journal: &EventJournal,
) -> HttpResponse {
    handle_webhook_request_inner(request, secret, journal, None)
}

/// Handle a delivery using a host-only receiver key in addition to the Forgejo
/// HMAC secret. Capabilities reads do not require either key.
pub fn handle_webhook_request_attested(
    request: &HttpRequest,
    secret: &[u8],
    journal: &EventJournal,
    signing_key: &SigningKey,
) -> HttpResponse {
    handle_webhook_request_inner(request, secret, journal, Some(signing_key))
}

fn handle_webhook_request_inner(
    request: &HttpRequest,
    secret: &[u8],
    journal: &EventJournal,
    attestation_signer: Option<&SigningKey>,
) -> HttpResponse {
    if request.target == "/forgejo/capabilities" {
        if request.method != "GET" {
            return error_response(
                405,
                "method_not_allowed",
                false,
                "read capabilities with GET /forgejo/capabilities",
            );
        }
        return capabilities_response();
    }
    if request.target != "/forgejo/webhook" {
        return error_response(404, "not_found", false, "use POST /forgejo/webhook");
    }
    if request.method != "POST" {
        return error_response(405, "method_not_allowed", false, "send a POST request");
    }
    let content_type = match request.single_header("content-type") {
        Ok(value) => value,
        Err(_) => {
            return error_response(415, "content_type_required", false, "send application/json");
        }
    };
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        return error_response(
            415,
            "unsupported_content_type",
            false,
            "send application/json",
        );
    }

    let delivery_id = match request.header_alias("x-forgejo-delivery", "x-gitea-delivery") {
        Ok(value) => value,
        Err(_) => {
            return error_response(
                400,
                "delivery_id_invalid",
                false,
                "send one matching Forgejo/Gitea delivery ID",
            );
        }
    };
    let event_name = match request.header_alias("x-forgejo-event", "x-gitea-event") {
        Ok(value) => value,
        Err(_) => {
            return error_response(
                400,
                "event_header_invalid",
                false,
                "send one matching Forgejo/Gitea event name",
            );
        }
    };
    let signature = match request.header_alias("x-forgejo-signature", "x-gitea-signature") {
        Ok(value) => value,
        Err(_) => {
            return error_response(
                401,
                "signature_missing",
                false,
                "check the webhook secret and signature headers",
            );
        }
    };

    let ingest_result = match attestation_signer {
        Some(signing_key) => ingest_forgejo_delivery_attested(
            journal,
            secret,
            signing_key,
            delivery_id,
            event_name,
            signature,
            &request.body,
        ),
        None => ingest_forgejo_delivery(
            journal,
            secret,
            delivery_id,
            event_name,
            signature,
            &request.body,
        ),
    };
    match ingest_result {
        Ok(AppendResult::Appended(event)) => HttpResponse {
            status: 202,
            retry_after_seconds: None,
            body: json!({
                "schema_version": 1,
                "accepted": true,
                "duplicate": false,
                "delivery_id": delivery_id,
                "sequence": event.sequence,
                "reconcile_required": true,
            }),
        },
        Ok(AppendResult::Duplicate(event)) => HttpResponse {
            status: 202,
            retry_after_seconds: None,
            body: json!({
                "schema_version": 1,
                "accepted": true,
                "duplicate": true,
                "delivery_id": delivery_id,
                "sequence": event.sequence,
                "reconcile_required": true,
            }),
        },
        Err(WebhookError::InvalidSignature) => error_response(
            401,
            "invalid_signature",
            false,
            "check the configured webhook secret",
        ),
        Err(WebhookError::UnsupportedEvent) => {
            let action = format!(
                "configure the Forgejo hook for one of [{}] and follow the matching webhook_ingest.event_contracts entry from GET /forgejo/capabilities",
                SUPPORTED_EVENTS.join(", ")
            );
            error_response(422, "unsupported_event", false, &action)
        }
        Err(WebhookError::UnsupportedEventPayloadField(field)) => HttpResponse {
            status: 422,
            retry_after_seconds: None,
            body: json!({
                "schema_version": 1,
                "accepted": false,
                "error": {
                    "code": "unsupported_event",
                    "retryable": false,
                    "action": "this adapter accepts branch refs only; exclude tag create/delete deliveries at a trusted sender-side filter, or disable those event subscriptions until explicit tag support is added; consult GET /forgejo/capabilities; the rejected body was not journaled",
                    "details": {"field_path": format!("payload.{field}")}
                }
            }),
        },
        Err(WebhookError::InvalidEventPayload) => error_response(
            400,
            "event_payload_invalid",
            false,
            "validate the signed payload against the matching event contract from GET /forgejo/capabilities; the rejected body was not journaled",
        ),
        Err(WebhookError::InvalidEventPayloadField(field)) => {
            let action = match field {
                "action" => {
                    "correct payload.action using the supported actions in the matching event contract from GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "is_pull" => {
                    "set payload.is_pull to a JSON boolean; true requires payload.pull_request.number to equal payload.issue.number, false requires payload.pull_request to be absent; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "comment.id" => {
                    "set payload.comment.id to a positive JSON integer; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "release.id" => {
                    "set payload.release.id to a positive JSON integer; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "repository.full_name" => {
                    "ask the trusted sender to provide a signed payload with repository.full_name set to a valid owner/repository pair consistent with repository.name and repository.owner.login; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "run.repository.full_name" => {
                    "ask the trusted sender to provide a signed payload with run.repository.full_name set to a valid owner/repository pair consistent with run.repository.name and run.repository.owner.login; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "pull_request.base.repo" => {
                    "ask the trusted sender to provide a signed payload whose pull_request.base.repo identity matches repository.full_name; head.repo may remain a contributor fork; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "pull_request.base" => {
                    "when supplied, set payload.pull_request.base to a JSON object; its repo identity must match repository.full_name while head.repo may remain a contributor fork; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "sha" => {
                    "set payload.sha to a nonzero 40- or 64-character hexadecimal commit ID; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "run.commit_sha" => {
                    "set payload.run.commit_sha to a nonzero 40- or 64-character hexadecimal commit ID; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "run.id" => {
                    "set payload.run.id to a positive JSON integer; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "run.status" => {
                    "compare the signed payload.run.status with X-Forgejo-Event and the workflow_run event contract from GET /forgejo/capabilities; ask the trusted sender or forwarder to correct an inconsistent header, then redeliver the original signed body with the same delivery ID; do not alter the signed body; the rejected body was not journaled"
                }
                "job.status" => {
                    "compare the signed payload.job.status with X-Forgejo-Event and the workflow_job event contract from GET /forgejo/capabilities; ask the trusted sender or forwarder to correct an inconsistent header, then redeliver the original signed body with the same delivery ID; do not alter the signed body; the rejected body was not journaled"
                }
                "run.index_in_repo" => {
                    "set payload.run.index_in_repo to a positive JSON integer; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "run.updated" => {
                    "set payload.run.updated to a valid RFC3339 timestamp with timezone suffix Z or ±HH:MM; leap seconds are not supported; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "ref" => {
                    "set payload.ref to a Git-valid branch name with no empty path component or component starting with dot, no trailing slash or dot, and no Git ref metacharacters; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                "ref_type" => {
                    "set payload.ref_type to branch for branch create/delete events; tag refs are not accepted by this adapter; consult GET /forgejo/capabilities and redeliver with the same delivery ID; the rejected body was not journaled"
                }
                _ => {
                    "inspect the event payload field against the event contract; preserve the journal and do not edit it directly"
                }
            };
            HttpResponse {
                status: 400,
                retry_after_seconds: None,
                body: json!({
                    "schema_version": 1,
                    "accepted": false,
                    "error": {
                        "code": "event_payload_invalid",
                        "retryable": false,
                        "action": action,
                        "details": {"field_path": format!("payload.{field}")}
                    }
                }),
            }
        }
        Err(WebhookError::InvalidJsonAt { line, column }) => HttpResponse {
            status: 400,
            retry_after_seconds: None,
            body: json!({
                "schema_version": 1,
                "accepted": false,
                "error": {
                    "code": "payload_invalid",
                    "retryable": false,
                    "action": "correct the JSON at its source using the reported line and column, then redeliver it with a valid signature; the rejected body was not journaled",
                    "details": {"line": line, "column": column}
                }
            }),
        },
        Err(WebhookError::IdempotencyConflict) => error_response(
            409,
            "delivery_conflict",
            false,
            "inspect the delivery ID reuse before retrying",
        ),
        Err(WebhookError::BodyTooLarge) => error_response(
            413,
            "body_too_large",
            false,
            "reduce the webhook payload below 1 MiB",
        ),
        Err(WebhookError::InvalidDeliveryId | WebhookError::InvalidJson) => error_response(
            400,
            "payload_invalid",
            false,
            "check the delivery ID and JSON body",
        ),
        Err(WebhookError::EmptySecret) => error_response(
            500,
            "verifier_misconfigured",
            false,
            "configure a non-empty webhook secret",
        ),
        Err(WebhookError::Journal(JournalError::InboxTooLarge { .. })) => error_response(
            507,
            "inbox_capacity_exceeded",
            false,
            "preserve a backup, apply a verified inbox compaction or migration, then manually redeliver if needed",
        ),
        Err(WebhookError::Journal(JournalError::LockBusy)) => HttpResponse {
            status: 503,
            retry_after_seconds: Some(JOURNAL_BUSY_RETRY_AFTER_SECONDS),
            body: json!({
                "schema_version": 1,
                "accepted": false,
                "error": {
                    "code": "journal_busy",
                    "retryable": true,
                    "action": "retry the exact signed delivery after the active inbox operation completes; do not edit the inbox"
                }
            }),
        },
        Err(WebhookError::Journal(_)) => error_response(
            503,
            "journal_unavailable",
            true,
            "check journal storage and retry delivery",
        ),
        Err(
            WebhookError::DeliveryNotFound
            | WebhookError::InvalidJournaledEvent
            | WebhookError::Attestation(_)
            | WebhookError::QuarantinedDelivery { .. },
        ) => error_response(
            500,
            "internal_error",
            false,
            "report the unexpected webhook ingestion error to the local operator",
        ),
    }
}

fn capabilities_response() -> HttpResponse {
    HttpResponse {
        status: 200,
        retry_after_seconds: None,
        body: json!({
            "schema_version": 9,
            "provider": "forgejo",
            "capability_endpoint": {
                "method": "GET",
                "path": "/forgejo/capabilities"
            },
            "webhook_ingest": {
                "method": "POST",
                "path": "/forgejo/webhook",
                "content_type": "application/json",
                "signature": {
                    "algorithm": "HMAC-SHA256",
                    "headers": ["X-Forgejo-Signature", "X-Gitea-Signature"]
                },
                "delivery_id_headers": ["X-Forgejo-Delivery", "X-Gitea-Delivery"],
                "event_headers": ["X-Forgejo-Event", "X-Gitea-Event"],
                "supported_events": SUPPORTED_EVENTS,
                "event_contracts": WEBHOOK_EVENT_CONTRACTS_V1,
                "event_contracts_v2": WEBHOOK_EVENT_CONTRACTS_V2,
                "event_contracts_v3": WEBHOOK_EVENT_CONTRACTS_V3,
                "event_contracts_v4": WEBHOOK_EVENT_CONTRACTS_V4,
                "event_contracts_v5": WEBHOOK_EVENT_CONTRACTS_V5,
                "event_contracts_v6": WEBHOOK_EVENT_CONTRACTS_V6,
                "event_contracts_v7": WEBHOOK_EVENT_CONTRACTS_V7,
                "event_contracts_v8": WEBHOOK_EVENT_CONTRACTS_V8,
                "event_contracts_v9": WEBHOOK_EVENT_CONTRACTS_V9,
                "idempotency": {
                    "same_delivery_and_payload": {"status": 202, "duplicate": true},
                    "same_delivery_with_different_payload": {
                        "status": 409,
                        "error_code": "delivery_conflict"
                    }
                }
            },
            "error_contract": {
                "schema_version": 2,
                "required_error_fields": ["code", "retryable", "action"],
                "optional_error_fields": ["details"],
                "event_payload_invalid": {
                    "status": 400,
                    "details_field_path": "payload.<field>"
                },
                "journal_busy": {
                    "status": 503,
                    "retryable": true,
                    "retry_after_seconds": JOURNAL_BUSY_RETRY_AFTER_SECONDS,
                    "action": "retry the exact signed delivery after the active inbox operation completes; do not edit the inbox"
                },
                "server_busy": {
                    "status": 503,
                    "retryable": true,
                    "retry_after_seconds": SERVER_BUSY_RETRY_AFTER_SECONDS,
                    "action": "retry the exact signed delivery after the receiver has drained its bounded request queue"
                },
                "malformed_json": {
                    "status": 400,
                    "error_code": "payload_invalid",
                    "details_fields": ["line", "column"]
                }
            },
            "limits": {
                "max_header_bytes": MAX_HEADER_BYTES,
                "max_body_bytes": MAX_WEBHOOK_BODY_BYTES
            }
        }),
    }
}

fn error_response(status: u16, code: &str, retryable: bool, action: &str) -> HttpResponse {
    HttpResponse {
        status,
        retry_after_seconds: None,
        body: json!({
            "schema_version": 1,
            "accepted": false,
            "error": {"code": code, "retryable": retryable, "action": action},
        }),
    }
}

fn is_header_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

/// Validate the ASCII URI-authority subset used by HTTP Host fields.
///
/// Host names are not used for routing by this loopback receiver, but invalid
/// syntax is rejected before a signed request can be journaled. International
/// names must use their ASCII/punycode form; scoped IPv6 zone identifiers are
/// intentionally outside this parser's accepted subset.
fn is_valid_host_authority(value: &str) -> bool {
    if value.is_empty() || !value.is_ascii() {
        return false;
    }

    if let Some(literal_and_port) = value.strip_prefix('[') {
        let Some((literal, suffix)) = literal_and_port.split_once(']') else {
            return false;
        };
        if !is_valid_ip_literal(literal) {
            return false;
        }
        return suffix.is_empty() || suffix.strip_prefix(':').is_some_and(is_valid_port_syntax);
    }

    if value.contains(['[', ']']) {
        return false;
    }
    let (host, port) = match value.split_once(':') {
        Some((host, port)) if !port.contains(':') => (host, Some(port)),
        Some(_) => return false,
        None => (value, None),
    };
    !host.is_empty() && is_valid_reg_name(host) && port.is_none_or(is_valid_port_syntax)
}

fn is_valid_ip_literal(value: &str) -> bool {
    value.parse::<std::net::Ipv6Addr>().is_ok() || is_valid_ipvfuture(value)
}

fn is_valid_ipvfuture(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 4 || !matches!(bytes[0], b'v' | b'V') {
        return false;
    }
    let Some(dot) = bytes.iter().position(|byte| *byte == b'.') else {
        return false;
    };
    if dot == 1 || !bytes[1..dot].iter().all(u8::is_ascii_hexdigit) {
        return false;
    }
    let address = &bytes[dot + 1..];
    !address.is_empty()
        && address
            .iter()
            .all(|byte| is_unreserved(*byte) || is_sub_delimiter(*byte) || *byte == b':')
}

fn is_valid_port_syntax(value: &str) -> bool {
    value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_valid_reg_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return false;
            }
            index += 3;
        } else if is_unreserved(bytes[index]) || is_sub_delimiter(bytes[index]) {
            index += 1;
        } else {
            return false;
        }
    }
    !bytes.is_empty()
}

fn is_unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)
}

fn is_sub_delimiter(byte: u8) -> bool {
    b"!$&'()*+,;=".contains(&byte)
}
