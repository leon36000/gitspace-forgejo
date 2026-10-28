use agent_forge_core::webhook::{
    MAX_WEBHOOK_BODY_BYTES,
    http::{HttpRequestError, parse_http_request},
};

fn request(headers: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes =
        format!("POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\n{headers}\r\n").into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

#[test]
fn parser_preserves_body_bytes_and_accepts_matching_forgejo_aliases() {
    let body = br#"{"action":"opened"}"#;
    let input = request(
        &format!(
            "Content-Length: {}\r\nContent-Type: application/json\r\nX-Forgejo-Delivery: id-1\r\nX-Gitea-Delivery: id-1\r\n",
            body.len()
        ),
        body,
    );

    let parsed = parse_http_request(&input).unwrap();

    assert_eq!(parsed.method, "POST");
    assert_eq!(parsed.target, "/forgejo/webhook");
    assert_eq!(parsed.body, body);
    assert_eq!(
        parsed
            .header_alias("x-forgejo-delivery", "x-gitea-delivery")
            .unwrap(),
        "id-1"
    );
}

#[test]
fn mismatched_forgejo_and_gitea_aliases_are_rejected() {
    let body = b"{}";
    let input = request(
        &format!(
            "Content-Length: {}\r\nX-Forgejo-Delivery: id-1\r\nX-Gitea-Delivery: id-2\r\n",
            body.len()
        ),
        body,
    );
    let parsed = parse_http_request(&input).unwrap();

    assert_eq!(
        parsed.header_alias("x-forgejo-delivery", "x-gitea-delivery"),
        Err(HttpRequestError::Malformed)
    );
}

#[test]
fn conflicting_content_length_and_chunked_transfer_are_rejected() {
    let duplicate = request("Content-Length: 2\r\nContent-Length: 2\r\n", b"{}");
    assert_eq!(
        parse_http_request(&duplicate),
        Err(HttpRequestError::Malformed)
    );

    let chunked = request("Transfer-Encoding: chunked\r\n", b"0\r\n\r\n");
    assert_eq!(
        parse_http_request(&chunked),
        Err(HttpRequestError::Malformed)
    );
}

#[test]
fn oversized_and_incomplete_requests_are_distinguished() {
    let oversized = request(
        &format!("Content-Length: {}\r\n", MAX_WEBHOOK_BODY_BYTES + 1),
        b"",
    );
    assert_eq!(
        parse_http_request(&oversized),
        Err(HttpRequestError::BodyTooLarge)
    );

    let incomplete =
        b"POST /forgejo/webhook HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 4\r\n\r\n{}";
    assert_eq!(
        parse_http_request(incomplete),
        Err(HttpRequestError::Incomplete)
    );
}

#[test]
fn absent_content_length_means_empty_body_and_rejects_undeclared_bytes() {
    let bodyless = b"GET /forgejo/capabilities HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n";
    assert_eq!(parse_http_request(bodyless).unwrap().body, b"");

    let undeclared_body = b"GET /forgejo/capabilities HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\nx";
    assert_eq!(
        parse_http_request(undeclared_body),
        Err(HttpRequestError::Malformed)
    );
}

#[test]
fn host_authority_accepts_uri_host_forms_and_rejects_malformed_syntax() {
    for host in [
        "forgejo.local",
        "forgejo.local:3000",
        "127.0.0.1",
        "[::1]",
        "[::1]:3130",
        "[v1.a]",
        "forgejo%2Elocal",
    ] {
        let input = String::from_utf8(request("Content-Length: 0\r\n", b""))
            .unwrap()
            .replace("Host: 127.0.0.1", &format!("Host: {host}"));
        assert!(
            parse_http_request(input.as_bytes()).is_ok(),
            "valid Host authority {host:?} should be accepted"
        );
    }

    for host in [
        "forgejo.local/path",
        "forgejo.local:abc",
        "[not-ipv6]",
        "[::1]suffix",
        "forgejo.local extra",
        "user@forgejo.local",
        "forgejo%2",
        "forgejó.local",
    ] {
        let input = String::from_utf8(request("Content-Length: 0\r\n", b""))
            .unwrap()
            .replace("Host: 127.0.0.1", &format!("Host: {host}"));
        assert_eq!(
            parse_http_request(input.as_bytes()),
            Err(HttpRequestError::Malformed),
            "malformed Host authority {host:?} should be rejected"
        );
    }
}

#[test]
fn duplicate_security_header_is_rejected() {
    let body = b"{}";
    let input = request(
        &format!(
            "Content-Length: {}\r\nX-Forgejo-Signature: a\r\nX-Forgejo-Signature: b\r\n",
            body.len()
        ),
        body,
    );
    let parsed = parse_http_request(&input).unwrap();

    assert_eq!(
        parsed.header_alias("x-forgejo-signature", "x-gitea-signature"),
        Err(HttpRequestError::Malformed)
    );
}
