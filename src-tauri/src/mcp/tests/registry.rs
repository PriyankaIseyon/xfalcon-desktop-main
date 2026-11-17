//! What this build will and will not dial.
//!
//! Replaces the earlier `no_custom_servers` suite: registering a server is now
//! a supported feature, so the invariant under test moved from "registration is
//! impossible" to "registration cannot widen what a URL is allowed to be, and
//! an unusable row is listed with a reason rather than silently dropped or
//! silently dialed."

use reqwest::Url;
use xfalcon_contracts::mcp::{
    McpAuthType, McpPermissionMode, McpServerUnavailableReason, McpSignInStatus, McpTransport,
    BUNDLED_SERVER_ID,
};

use crate::clients::da::mcp_servers::McpServerRecord;
use crate::mcp::auth::RegisteredAuth;
use crate::mcp::registry::{
    bundled_view, classify as classify_records, host_is_dialable_with, host_looks_private,
    is_bundled_alias, name_is_registrable, slug_for, validate_endpoint, validate_endpoint_with,
    DeviceOauthClient, DialableServer, EndpointError, ServerAuthRequirement, ServerVerdict,
    ENFORCE_PUBLIC_MCP_HOSTS,
};

/// URLs an on-prem or private deployment runs its MCP server on. The shipped
/// policy (`ENFORCE_PUBLIC_MCP_HOSTS` off) accepts every one of them.
const PRIVATE_DEPLOYMENT_URLS: &[&str] = &[
    "https://localhost/mcp",
    "https://127.0.0.1:8443/mcp",
    "https://10.0.0.5/mcp",
    "https://192.168.1.20:9000/mcp",
    "https://[::1]/mcp",
    "https://[fd00::1]/mcp",
    "https://[::ffff:10.0.0.5]/mcp",
    "http://localhost:3000/mcp",
    "http://127.0.0.1:8080/mcp",
    "http://10.0.0.5:8080/mcp",
    "http://[fd00::1]:8080/mcp",
    "http://mcp-server:8080/mcp",
    "http://mcp.corp/mcp",
    "https://mcp.example.com/mcp",
];

fn record(name: &str, url: Option<&str>) -> McpServerRecord {
    McpServerRecord {
        id: format!("id-{name}"),
        name: name.to_owned(),
        display_name: None,
        description: None,
        transport: McpTransport::Http,
        url: url.map(str::to_owned),
        auth_type: McpAuthType::None,
        permission_mode: McpPermissionMode::Ask,
        enabled: true,
        header_name: None,
        auth_config: None,
    }
}

fn always_credentialed(_id: &str) -> bool {
    true
}

fn always_signed_in(_id: &str) -> McpSignInStatus {
    McpSignInStatus::SignedIn
}

fn signed_in_for_account(_id: &str) -> McpSignInStatus {
    McpSignInStatus::SignedInForAccount
}

fn never_signed_in(_id: &str) -> McpSignInStatus {
    McpSignInStatus::NotSignedIn
}

fn never_credentialed(_id: &str) -> bool {
    false
}

/// No device holds an operator-issued client, which is the common case: most
/// servers register one on request. The tests that care supply their own.
fn no_oauth_client(_id: &str) -> Option<DeviceOauthClient> {
    None
}

fn classify(
    records: &[McpServerRecord],
    has_device_credential: &dyn Fn(&str) -> bool,
    sign_in_state: &dyn Fn(&str) -> McpSignInStatus,
) -> Vec<(xfalcon_contracts::mcp::McpServerView, ServerVerdict)> {
    classify_with_client(
        records,
        has_device_credential,
        sign_in_state,
        &no_oauth_client,
    )
}

fn classify_with_client(
    records: &[McpServerRecord],
    has_device_credential: &dyn Fn(&str) -> bool,
    sign_in_state: &dyn Fn(&str) -> McpSignInStatus,
    oauth_client: &dyn Fn(&str) -> Option<DeviceOauthClient>,
) -> Vec<(xfalcon_contracts::mcp::McpServerView, ServerVerdict)> {
    classify_records(
        records,
        &Url::parse("https://tenant.example/mcp").expect("bundled endpoint"),
        has_device_credential,
        sign_in_state,
        oauth_client,
    )
}

fn dialable(verdict: &ServerVerdict) -> &DialableServer {
    match verdict {
        ServerVerdict::Dialable(server) => server,
        ServerVerdict::Unavailable(reason) => panic!("expected dialable, got {reason:?}"),
    }
}

#[test]
fn an_https_endpoint_is_dialed_exactly_as_registered() {
    let endpoint =
        validate_endpoint("https://mcp.example.test/some/path").expect("a plain https URL");
    // No `/mcp` is appended: a registered server's path is its own.
    assert_eq!(endpoint.as_str(), "https://mcp.example.test/some/path");
}

#[test]
fn a_non_https_scheme_is_refused() {
    // Not `http` and not `https`, so refused in every build and under either
    // policy.
    for raw in ["ftp://mcp.example.test/", "file:///etc/passwd", "not a url"] {
        assert_eq!(
            validate_endpoint_with(raw, true),
            Err(EndpointError::NotHttps),
            "{raw} should not be dialable"
        );
        assert_eq!(
            validate_endpoint_with(raw, false),
            Err(EndpointError::NotHttps),
            "{raw} should not be dialable"
        );
    }
}

#[test]
fn an_endpoint_carrying_credentials_or_a_query_is_refused() {
    assert_eq!(
        validate_endpoint("https://user:pass@mcp.example.test/"),
        Err(EndpointError::EmbeddedUserinfo)
    );
    assert_eq!(
        validate_endpoint("https://mcp.example.test/?token=stolen"),
        Err(EndpointError::QueryOrFragment)
    );
    assert_eq!(
        validate_endpoint("https://mcp.example.test/#fragment"),
        Err(EndpointError::QueryOrFragment)
    );
}

#[test]
fn an_enforced_policy_refuses_private_or_unroutable_hosts_in_every_build() {
    // Enforcing policy: these are refused only while the public-hosts rules are
    // on. The shipped policy accepts them, see
    // `the_default_policy_accepts_private_deployment_urls`.
    for raw in [
        "https://10.0.0.5/mcp",
        "https://192.168.1.10/mcp",
        "https://172.16.0.1/mcp",
        "https://169.254.169.254/latest",
        "https://0.0.0.0/mcp",
        "https://[fc00::1]/mcp",
        "https://[fe80::1]/mcp",
    ] {
        assert_eq!(
            validate_endpoint_with(raw, true),
            Err(EndpointError::HostRejected),
            "{raw} must never be dialable"
        );
    }
}

/// The same destinations as the test above, spelled so that no `Ipv6Addr` range
/// predicate recognises them.
///
/// `::ffff:a.b.c.d` is global unicast at the IPv6 layer — `is_loopback` is
/// `false` for `::ffff:127.0.0.1` — and `64:ff9b::/96` hands the IPv4 address to
/// a NAT64 translator to dial. Both would have carried the device credential to
/// a host inside the user's own network while every rule above passed.
#[test]
fn an_ipv4_destination_cannot_be_smuggled_through_an_ipv6_literal() {
    for raw in [
        "https://[::ffff:10.0.0.5]/mcp",
        "https://[::ffff:192.168.1.10]/mcp",
        "https://[::ffff:169.254.169.254]/latest",
        "https://[::127.0.0.1]/mcp",
        "https://[64:ff9b::a00:5]/mcp",
        "https://[64:ff9b:1::a9fe:a9fe]/mcp",
    ] {
        assert_eq!(
            validate_endpoint_with(raw, true),
            Err(EndpointError::HostRejected),
            "{raw} must never be dialable"
        );
    }
}

/// Carrier-grade NAT space (RFC 6598) is the subscriber side of a consumer ISP's
/// network, so it is reachable from the desktop for the same reason 10/8 is.
#[test]
fn carrier_grade_nat_space_is_refused_and_its_neighbours_are_not() {
    for raw in ["https://100.64.0.1/mcp", "https://100.127.255.254/mcp"] {
        assert_eq!(
            validate_endpoint_with(raw, true),
            Err(EndpointError::HostRejected),
            "{raw} must never be dialable"
        );
    }
    for raw in ["https://100.63.255.255/mcp", "https://100.128.0.1/mcp"] {
        assert!(
            validate_endpoint_with(raw, true).is_ok(),
            "{raw} is outside 100.64.0.0/10 and must stay dialable"
        );
    }
}

/// Both spellings of loopback, because they must follow the *same* profile:
/// `::ffff:127.0.0.1` reaching the loopback interface in a release build is the
/// bypass, not a separate feature.
#[test]
fn loopback_follows_the_build_profile() {
    for raw in [
        "https://127.0.0.1:8931/mcp",
        "https://[::1]:8931/mcp",
        "https://[::ffff:127.0.0.1]:8931/mcp",
    ] {
        let outcome = validate_endpoint_with(raw, true);
        if cfg!(debug_assertions) {
            assert!(
                outcome.is_ok(),
                "a debug build must let a developer dial a local MCP server at {raw}"
            );
        } else {
            assert_eq!(outcome, Err(EndpointError::HostRejected), "{raw}");
        }
    }
}

#[test]
fn a_public_host_is_dialable() {
    // Public hosts pass under either policy.
    for enforce in [true, false] {
        assert!(validate_endpoint_with("https://mcp.example.test/mcp", enforce).is_ok());
        assert!(validate_endpoint_with("https://203.0.113.10/mcp", enforce).is_ok());
    }
}

#[test]
fn the_default_policy_accepts_private_deployment_urls() {
    // Asserts the shipped policy through the public function, so it applies only
    // while the switch is off. `the_permissive_policy_accepts_private_deployment_urls`
    // covers the same URLs in both modes.
    if ENFORCE_PUBLIC_MCP_HOSTS {
        return;
    }
    for raw in PRIVATE_DEPLOYMENT_URLS {
        assert!(
            validate_endpoint(raw).is_ok(),
            "{raw} must be dialable under the shipped policy"
        );
    }
}

#[test]
fn the_permissive_policy_accepts_private_deployment_urls() {
    for raw in PRIVATE_DEPLOYMENT_URLS {
        assert!(
            validate_endpoint_with(raw, false).is_ok(),
            "{raw} must be dialable when public hosts are not enforced"
        );
    }
}

#[test]
fn the_public_entry_point_follows_the_switch() {
    // Holds whatever the constant is: the public function must pass the switch
    // through, not a value of its own.
    for raw in PRIVATE_DEPLOYMENT_URLS
        .iter()
        .chain(&["https://mcp.example.com/mcp", "http://mcp.example.com/mcp"])
    {
        assert_eq!(
            validate_endpoint(raw),
            validate_endpoint_with(raw, ENFORCE_PUBLIC_MCP_HOSTS),
            "{raw}"
        );
    }
}

#[test]
fn a_public_cleartext_host_is_refused_outside_debug_builds_when_not_enforced() {
    // Cleartext is a debug-build allowance for any host, and under the permissive
    // policy the only other cleartext is to a private host.
    for raw in ["http://mcp.example.com/mcp", "http://203.0.113.10/mcp"] {
        let outcome = validate_endpoint_with(raw, false);
        if cfg!(debug_assertions) {
            assert!(outcome.is_ok(), "{raw} is accepted in a debug build");
        } else {
            assert_eq!(outcome, Err(EndpointError::NotHttps), "{raw}");
        }
    }
}

#[test]
fn malformed_urls_are_refused_under_either_policy() {
    // Malformed-URL guards are not deployment policy, so every case here is
    // refused by the public function and by both policies.
    let cases = [
        (
            "https://user:pw@10.0.0.5/mcp",
            EndpointError::EmbeddedUserinfo,
        ),
        ("https://10.0.0.5/mcp?x=1", EndpointError::QueryOrFragment),
        ("https://10.0.0.5/mcp#f", EndpointError::QueryOrFragment),
        ("ftp://10.0.0.5/mcp", EndpointError::NotHttps),
    ];
    for (raw, expected) in cases {
        assert_eq!(validate_endpoint(raw), Err(expected), "{raw}");
        assert_eq!(validate_endpoint_with(raw, true), Err(expected), "{raw}");
        assert_eq!(validate_endpoint_with(raw, false), Err(expected), "{raw}");
    }
    // `https://` does not parse, which is reported as NotHttps. A missing host
    // on an http(s) URL cannot reach the host check: the URL parser always
    // gives one.
    assert_eq!(validate_endpoint("https://"), Err(EndpointError::NotHttps));
    assert_eq!(
        validate_endpoint_with("https://", false),
        Err(EndpointError::NotHttps)
    );
}

#[test]
fn the_enforcing_policy_refuses_private_https_hosts_in_every_build() {
    for raw in [
        "https://10.0.0.5/mcp",
        "https://192.168.1.20:9000/mcp",
        "https://[fd00::1]/mcp",
        "https://[::ffff:10.0.0.5]/mcp",
        "https://100.64.0.1/mcp",
    ] {
        assert_eq!(
            validate_endpoint_with(raw, true),
            Err(EndpointError::HostRejected),
            "{raw} must be refused when public hosts are enforced"
        );
    }
    assert!(validate_endpoint_with("https://mcp.example.com/mcp", true).is_ok());
}

#[test]
fn the_enforcing_policy_refuses_cleartext_outside_debug_builds() {
    // (raw, dialable in a debug build under the enforcing policy). In a release
    // build cleartext is refused for every host.
    let cases = [
        ("http://localhost:3000/mcp", true),
        ("http://127.0.0.1:8080/mcp", true),
        ("http://10.0.0.5:8080/mcp", false),
        ("http://[fd00::1]:8080/mcp", false),
        ("http://mcp-server:8080/mcp", true),
        ("http://mcp.example.com/mcp", true),
    ];
    for (raw, debug_dialable) in cases {
        let outcome = validate_endpoint_with(raw, true);
        if cfg!(debug_assertions) {
            assert_eq!(outcome.is_ok(), debug_dialable, "{raw} in a debug build");
        } else {
            assert_eq!(
                outcome,
                Err(EndpointError::NotHttps),
                "{raw} in a release build"
            );
        }
    }
}

#[test]
fn host_looks_private_judges_the_spelling_of_a_host() {
    let cases = [
        // On the user's own network by name.
        ("localhost", true),
        ("localhost.", true),
        ("LOCALHOST", true),
        ("box.localhost", true),
        ("minio", true),
        ("mcp-server", true),
        ("a.b.local", true),
        ("Printer.LOCAL", true),
        ("db.internal", true),
        ("mcp.corp", true),
        ("files.lan", true),
        ("wiki.intranet", true),
        ("host.localdomain", true),
        ("printer.home.arpa", true),
        // Non-public IP literals, bracketed or not.
        ("10.0.0.5", true),
        ("127.0.0.1", true),
        ("169.254.169.254", true),
        ("100.64.0.1", true),
        ("0.0.0.0", true),
        ("[::1]", true),
        ("::1", true),
        ("[fd00::1]", true),
        ("::ffff:10.0.0.5", true),
        // Public-looking names and public addresses are not private by spelling.
        ("mcp.example.com", false),
        ("mcp.example.com.", false),
        ("MCP.Example.com", false),
        ("notlocal.example.com", false),
        ("203.0.113.10", false),
        ("93.184.216.34", false),
        ("2606:2800:220:1::46", false),
    ];
    for (host, private) in cases {
        assert_eq!(host_looks_private(host), private, "{host}");
    }
}

#[test]
fn address_classification_agrees_with_the_literal_host_gate() {
    use crate::mcp::registry::address_is_dialable;
    use std::net::IpAddr;

    let literals = [
        "93.184.216.34",
        "10.0.0.5",
        "169.254.169.254",
        "100.64.0.1",
        "127.0.0.1",
        "0.0.0.0",
        "224.0.0.1",
        "2606:2800:220:1::46",
        "fd00::1",
        "fe80::1",
        "64:ff9b::a00:5",
        "::ffff:10.0.0.5",
        "::ffff:127.0.0.1",
        "::1",
        "::",
    ];
    for literal in literals {
        let address: IpAddr = literal.parse().expect("literal");
        assert_eq!(
            host_is_dialable_with(literal, true),
            address_is_dialable(address, cfg!(debug_assertions)),
            "{literal}"
        );
        assert_eq!(
            host_is_dialable_with(&format!("[{literal}]"), true),
            address_is_dialable(address, cfg!(debug_assertions)),
            "[{literal}]"
        );
        // Permissive: every host is dialable, whatever its classification.
        assert!(host_is_dialable_with(literal, false), "{literal}");
    }
    assert!(address_is_dialable(
        "127.0.0.1".parse().expect("literal"),
        true
    ));
    assert!(!address_is_dialable(
        "127.0.0.1".parse().expect("literal"),
        false
    ));
    assert!(!address_is_dialable("::1".parse().expect("literal"), false));
}

#[test]
fn an_address_that_carries_ipv4_is_judged_by_the_ipv4_it_carries() {
    use crate::mcp::registry::address_is_dialable;
    use std::net::IpAddr;

    // Expected answers are written out, not derived from the code under test.
    let cases = [
        // A DNS64 resolver answers a public IPv4-only server this way.
        ("64:ff9b::5db8:d822", true),
        ("64:ff9b::a00:5", false),
        ("64:ff9b::7f00:1", false),
        ("64:ff9b::a9fe:a9fe", false),
        // Local-use NAT64 space is refused whatever it carries.
        ("64:ff9b:1::5db8:d822", false),
        ("::ffff:0:5db8:d822", true),
        ("::ffff:0:a00:5", false),
        ("2002:5db8:d822::1", true),
        ("2002:a00:5::1", false),
        ("2002:7f00:1::1", false),
        // 0.0.0.0/8 is "this network", not a public host.
        ("0.1.2.3", false),
        ("0.0.0.5", false),
        ("93.184.216.34", true),
    ];
    for (literal, dialable) in cases {
        let address: IpAddr = literal.parse().expect("literal");
        assert_eq!(address_is_dialable(address, false), dialable, "{literal}");
    }
}

#[test]
fn a_trailing_dot_does_not_hide_a_private_literal_from_the_enforcing_policy() {
    use crate::mcp::registry::host_is_dialable_with;

    for host in ["10.0.0.5.", "192.168.1.1.", "169.254.169.254."] {
        assert!(!host_is_dialable_with(host, true), "{host} must be refused");
        assert!(
            host_is_dialable_with(host, false),
            "{host} is allowed while off"
        );
    }
    assert!(host_is_dialable_with("93.184.216.34.", true));
}

#[test]
fn a_registered_server_requires_public_hosts_and_the_bundled_one_does_not() {
    use crate::auth::AuthConfig;
    use crate::mcp::server_resolver::{McpServerDescriptor, McpServerResolver};

    // A registered server follows the switch. The enforcing case is the same
    // expression with `ENFORCE_PUBLIC_MCP_HOSTS` on: `registered()` reads the
    // constant, so there is no second value to keep in step with it.
    let registered = McpServerDescriptor::registered(
        Url::parse("https://mcp.example.test/mcp").expect("endpoint"),
    );
    assert_eq!(registered.public_hosts_only(), ENFORCE_PUBLIC_MCP_HOSTS);

    let config = AuthConfig {
        da_base_url: Url::parse("https://tenant.example").expect("da origin"),
        casdoor_issuer: Url::parse("https://auth.example").expect("issuer"),
        client_id: "client".to_owned(),
        organization: "org".to_owned(),
        loopback_port: 19847,
        mcp_client_id: None,
        mcp_client_secret: None,
    };
    assert!(!McpServerResolver::resolve(&config).public_hosts_only());
}

#[test]
fn a_remote_row_at_the_normalized_bundled_endpoint_is_an_alias() {
    let bundled = Url::parse("https://tenant.example/mcp").expect("bundled endpoint");
    let mut alias = record("workspace-alias", Some("https://TENANT.example:443/mcp"));

    assert!(is_bundled_alias(&alias, &bundled));
    assert!(classify_records(
        &[alias.clone()],
        &bundled,
        &always_credentialed,
        &always_signed_in,
        &no_oauth_client,
    )
    .is_empty());

    alias.url = Some("https://tenant.example/mcp/".to_owned());
    assert!(
        !is_bundled_alias(&alias, &bundled),
        "a distinct path must not be collapsed into the tenant endpoint"
    );
    alias.transport = McpTransport::Stdio;
    alias.url = Some(bundled.to_string());
    assert!(
        !is_bundled_alias(&alias, &bundled),
        "a local-process row is not an alias of the remote tenant connection"
    );
}

#[test]
fn a_slug_is_derived_bounded_and_never_empty() {
    assert_eq!(slug_for("Acme CRM"), "acme-crm");
    assert_eq!(slug_for("  spaced  out  "), "spaced-out");
    assert_eq!(slug_for("UPPER_case.dots"), "upper-case-dots");
    assert_eq!(slug_for("!!!"), "server");
    assert_eq!(slug_for("").len(), "server".len());
    assert!(slug_for(&"n".repeat(200)).len() <= 24);
    // A trailing separator must not survive into a tool id.
    assert!(!slug_for("trailing---").ends_with('-'));
}

#[test]
fn a_slug_never_contains_the_separator_a_tool_id_is_split_on() {
    // `crate::mcp::runtime::mcp_tool_route` recovers the server from
    // `mcp.<slug>.<tool>` by splitting at the first `.`, so a slug carrying one
    // would make every tool of that server unroutable.
    for name in [
        "dots.in.name",
        "Acme. CRM.",
        "..",
        "tabs\tand\nnewlines",
        "\u{202e}unicode\u{202d}",
        &"n".repeat(200),
    ] {
        let slug = slug_for(name);
        assert!(!slug.contains('.'), "{name:?} produced {slug:?}");
        assert!(!slug.is_empty(), "{name:?} produced an empty slug");
    }
}

#[test]
fn the_auth_requirement_the_runtime_binds_is_the_one_the_row_declared() {
    // `crate::mcp::auth` sits below this module and mirrors the vocabulary
    // rather than importing it; this pins the two together.
    assert_eq!(
        RegisteredAuth::from(&ServerAuthRequirement::None),
        RegisteredAuth::None
    );
    assert_eq!(
        RegisteredAuth::from(&ServerAuthRequirement::Bearer),
        RegisteredAuth::Bearer
    );
    assert_eq!(
        RegisteredAuth::from(&ServerAuthRequirement::Header {
            name: "X-Acme-Key".to_owned()
        }),
        RegisteredAuth::Header {
            name: "X-Acme-Key".to_owned()
        }
    );
}

#[test]
fn the_bundled_slug_is_reserved_against_registration() {
    assert!(!name_is_registrable(BUNDLED_SERVER_ID));
    assert!(!name_is_registrable("TENANT"));
    assert!(!name_is_registrable("  tenant  "));
    assert!(!name_is_registrable(""));
    assert!(!name_is_registrable(&"n".repeat(101)));
    assert!(name_is_registrable("Acme CRM"));
}

#[test]
fn colliding_names_get_distinct_namespaces_rather_than_shadowing_each_other() {
    let records = vec![
        record("Acme CRM", Some("https://a.example.test/mcp")),
        record("acme-crm", Some("https://b.example.test/mcp")),
        record("ACME  CRM", Some("https://c.example.test/mcp")),
    ];
    let classified = classify(&records, &always_credentialed, &always_signed_in);
    let slugs: Vec<&str> = classified
        .iter()
        .map(|(view, _)| view.slug.as_str())
        .collect();
    assert_eq!(slugs, vec!["acme-crm", "acme-crm-2", "acme-crm-3"]);
    // And the verdicts carry the same slugs the views advertise.
    for (view, verdict) in &classified {
        assert_eq!(dialable(verdict).slug, view.slug);
    }
}

#[test]
fn a_registered_row_can_never_claim_the_bundled_namespace() {
    let records = vec![record("tenant", Some("https://impostor.example.test/mcp"))];
    let classified = classify(&records, &always_credentialed, &always_signed_in);
    assert_ne!(classified[0].0.slug, BUNDLED_SERVER_ID);
    assert_eq!(classified[0].0.slug, "tenant-2");
}

/// A row labelled `sse` is dialled, not refused: the label is written by clients
/// that do not distinguish it from Streamable HTTP (see `verdict_for`), so
/// refusing it refused every server registered from xFalcon Web.
#[test]
fn a_row_naming_a_remote_http_endpoint_is_dialable_under_either_label() {
    for transport in [McpTransport::Http, McpTransport::Sse] {
        let records = vec![McpServerRecord {
            transport,
            ..record("remote", Some("https://mcp.example.test/mcp"))
        }];
        let classified = classify(&records, &always_credentialed, &always_signed_in);
        assert_eq!(
            classified[0].0.unavailable, None,
            "{transport:?} names a remote HTTP endpoint and must be attempted"
        );
        assert_eq!(
            dialable(&classified[0].1).descriptor.endpoint().as_str(),
            "https://mcp.example.test/mcp"
        );
    }
}

#[test]
fn an_unsupported_transport_is_listed_with_a_reason_not_dropped() {
    for transport in [McpTransport::Stdio, McpTransport::Other] {
        let records = vec![McpServerRecord {
            transport,
            ..record("local", Some("https://mcp.example.test/mcp"))
        }];
        let classified = classify(&records, &always_credentialed, &always_signed_in);
        assert_eq!(classified.len(), 1, "the row must still be listed");
        assert_eq!(
            classified[0].0.unavailable,
            Some(McpServerUnavailableReason::UnsupportedTransport)
        );
        assert!(matches!(classified[0].1, ServerVerdict::Unavailable(_)));
    }
}

#[test]
fn an_unsupported_auth_method_is_listed_with_a_reason() {
    let records = vec![McpServerRecord {
        auth_type: McpAuthType::Other,
        ..record("mystery", Some("https://mcp.example.test/mcp"))
    }];
    let classified = classify(&records, &always_credentialed, &always_signed_in);
    assert_eq!(
        classified[0].0.unavailable,
        Some(McpServerUnavailableReason::UnsupportedAuthMethod)
    );
}

/// An `oauth` row is dialable exactly when this device holds a grant, and says
/// which of the two it is — the surface offers "Sign in" or "Sign out" from that
/// field alone.
#[test]
fn an_oauth_row_is_dialable_once_this_device_has_signed_in() {
    let records = vec![McpServerRecord {
        auth_type: McpAuthType::Oauth,
        ..record("oauthy", Some("https://mcp.example.test/mcp"))
    }];

    let signed_out = classify(&records, &never_credentialed, &never_signed_in);
    assert_eq!(
        signed_out[0].0.unavailable,
        Some(McpServerUnavailableReason::NeedsSignIn),
        "an oauth row with no grant is recoverable, not unsupported"
    );
    assert_eq!(signed_out[0].0.sign_in, Some(McpSignInStatus::NotSignedIn));

    // The credential store is deliberately empty here: an oauth server needs a
    // grant, and a pasted credential is not a substitute for one.
    let signed_in = classify(&records, &never_credentialed, &always_signed_in);
    assert_eq!(signed_in[0].0.unavailable, None);
    assert_eq!(signed_in[0].0.sign_in, Some(McpSignInStatus::SignedIn));

    // The account-wide grant — the one xFalcon on the web established — makes the
    // row equally dialable, and is reported as its own state so the surface can
    // warn that signing out affects every client.
    let shared = classify(&records, &never_credentialed, &signed_in_for_account);
    assert_eq!(shared[0].0.unavailable, None);
    assert_eq!(
        shared[0].0.sign_in,
        Some(McpSignInStatus::SignedInForAccount)
    );
    assert!(matches!(shared[0].1, ServerVerdict::Dialable(_)));
    match &signed_in[0].1 {
        ServerVerdict::Dialable(server) => {
            assert_eq!(server.auth, ServerAuthRequirement::Oauth);
        }
        ServerVerdict::Unavailable(reason) => panic!("expected a dialable server, got {reason:?}"),
    }
}

/// Sign-in state is reported for `oauth` rows and nothing else, so a surface
/// cannot offer a sign-in for a server that has none to run.
#[test]
fn no_other_auth_method_reports_a_sign_in_state() {
    for auth_type in [
        McpAuthType::None,
        McpAuthType::Bearer,
        McpAuthType::Header,
        McpAuthType::Other,
    ] {
        let records = vec![McpServerRecord {
            auth_type,
            header_name: Some("X-Api-Key".to_owned()),
            ..record("plain", Some("https://mcp.example.test/mcp"))
        }];
        let classified = classify(&records, &always_credentialed, &always_signed_in);
        assert_eq!(
            classified[0].0.sign_in, None,
            "{auth_type:?} has no per-server sign-in"
        );
    }
}

#[test]
fn a_header_authenticated_row_without_a_header_name_is_not_dialable() {
    let records = vec![McpServerRecord {
        auth_type: McpAuthType::Header,
        header_name: None,
        ..record("keyed", Some("https://mcp.example.test/mcp"))
    }];
    let classified = classify(&records, &always_credentialed, &always_signed_in);
    assert_eq!(
        classified[0].0.unavailable,
        Some(McpServerUnavailableReason::UnsupportedAuthMethod)
    );
}

#[test]
fn a_row_needing_a_credential_this_device_lacks_is_listed_but_never_dialed() {
    for auth_type in [McpAuthType::Bearer, McpAuthType::Header] {
        let records = vec![McpServerRecord {
            auth_type,
            header_name: Some("X-Acme-Key".to_owned()),
            ..record("remote", Some("https://mcp.example.test/mcp"))
        }];

        let classified = classify(&records, &never_credentialed, &never_signed_in);
        assert_eq!(
            classified[0].0.unavailable,
            Some(McpServerUnavailableReason::MissingDeviceCredential),
            "{auth_type:?} without a device credential must not be dialable"
        );

        // The same row becomes dialable once the device holds one.
        let credentialed = classify(&records, &always_credentialed, &always_signed_in);
        assert_eq!(credentialed[0].0.unavailable, None);
    }
}

#[test]
fn an_open_server_needs_no_credential() {
    let records = vec![record("open", Some("https://mcp.example.test/mcp"))];
    let classified = classify(&records, &never_credentialed, &never_signed_in);
    assert_eq!(classified[0].0.unavailable, None);
    assert_eq!(dialable(&classified[0].1).auth, ServerAuthRequirement::None);
}

#[test]
fn the_auth_requirement_reaching_the_runtime_matches_the_row() {
    let records = vec![
        McpServerRecord {
            auth_type: McpAuthType::Bearer,
            ..record("bearer", Some("https://b.example.test/mcp"))
        },
        McpServerRecord {
            auth_type: McpAuthType::Header,
            header_name: Some("X-Acme-Key".to_owned()),
            ..record("header", Some("https://h.example.test/mcp"))
        },
    ];
    let classified = classify(&records, &always_credentialed, &always_signed_in);
    assert_eq!(
        dialable(&classified[0].1).auth,
        ServerAuthRequirement::Bearer
    );
    assert_eq!(
        dialable(&classified[1].1).auth,
        ServerAuthRequirement::Header {
            name: "X-Acme-Key".to_owned()
        }
    );
}

#[test]
fn a_missing_or_unusable_url_is_listed_with_the_url_reason() {
    // `ftp` is refused under either policy. A cleartext `http` URL is not used
    // here because the debug-build allowance accepts it for any host.
    for url in [
        None,
        Some("ftp://mcp.example.test/mcp"),
        Some("https://user:p@x/"),
    ] {
        let records = vec![record("broken", url)];
        let classified = classify(&records, &always_credentialed, &always_signed_in);
        assert_eq!(
            classified[0].0.unavailable,
            Some(McpServerUnavailableReason::InvalidUrl),
            "url {url:?} should be reported as invalid"
        );
    }
}

#[test]
fn a_disabled_row_is_still_listed_so_it_can_be_re_enabled() {
    let records = vec![McpServerRecord {
        enabled: false,
        ..record("paused", Some("https://mcp.example.test/mcp"))
    }];
    let classified = classify(&records, &always_credentialed, &always_signed_in);
    assert_eq!(classified.len(), 1);
    assert!(!classified[0].0.enabled);
}

#[test]
fn the_bundled_view_is_builtin_and_holds_the_reserved_slug() {
    let view = bundled_view("https://tenant.example.test/mcp");
    assert!(view.builtin);
    assert!(view.id.is_bundled());
    assert_eq!(view.slug, BUNDLED_SERVER_ID);
    assert_eq!(view.unavailable, None);
    assert_eq!(view.transport, McpTransport::Http);
}
