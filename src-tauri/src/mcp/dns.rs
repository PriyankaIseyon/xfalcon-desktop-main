//! DNS resolution for user-registered MCP servers and their OAuth endpoints.
//!
//! `registry::host_is_dialable_with` checks IP-literal hosts before any connection.
//! A DNS name cannot be judged until it resolves, so [`PublicOnlyResolver`]
//! performs that check at connection time: reqwest asks it for the name's
//! addresses, and it returns exactly the addresses it checked. reqwest then
//! connects only to those, so the checked answer is the dialled one.
//!
//! This resolver is used only while `registry::ENFORCE_PUBLIC_MCP_HOSTS` is
//! true. With that switch off, registered servers and the OAuth client resolve
//! names through the system resolver, because their servers may be on a private
//! address.

use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;

use reqwest::dns::{Addrs, Name, Resolve, Resolving};

use super::registry::address_is_dialable;

/// The lookup a resolver performs for one host name.
type LookupFuture = Pin<Box<dyn Future<Output = io::Result<Vec<SocketAddr>>> + Send>>;

/// A resolver that refuses any name with a non-public address in its answer.
///
/// Strict refusal rather than filtering: a name that answers with both public
/// and private addresses is a rebinding signal, so the whole name is refused.
/// Installed on the MCP and OAuth clients only while
/// `registry::ENFORCE_PUBLIC_MCP_HOSTS` is true.
pub(crate) struct PublicOnlyResolver {
    lookup: Box<dyn Fn(String) -> LookupFuture + Send + Sync>,
    allow_loopback: bool,
}

impl PublicOnlyResolver {
    /// Resolves through the system resolver. Loopback is dialable only in
    /// debug builds, matching `registry::host_is_dialable_with`.
    pub(crate) fn new() -> Self {
        Self::with_lookup(
            |host| {
                Box::pin(async move {
                    let addresses = tokio::net::lookup_host((host.as_str(), 0)).await?;
                    Ok(addresses.collect())
                })
            },
            cfg!(debug_assertions),
        )
    }

    fn with_lookup<F>(lookup: F, allow_loopback: bool) -> Self
    where
        F: Fn(String) -> LookupFuture + Send + Sync + 'static,
    {
        Self {
            lookup: Box::new(lookup),
            allow_loopback,
        }
    }
}

impl Resolve for PublicOnlyResolver {
    fn resolve(&self, name: Name) -> Resolving {
        let lookup = (self.lookup)(name.as_str().to_owned());
        let allow_loopback = self.allow_loopback;
        Box::pin(async move {
            let addresses = lookup.await?;
            checked_addresses(addresses, allow_loopback)
        })
    }
}

/// Accepts the answer only if it is non-empty and every address is dialable.
fn checked_addresses(
    addresses: Vec<SocketAddr>,
    allow_loopback: bool,
) -> Result<Addrs, Box<dyn std::error::Error + Send + Sync>> {
    if addresses.is_empty() {
        return Err("the name resolved to no addresses".into());
    }
    if addresses
        .iter()
        .any(|address| !address_is_dialable(address.ip(), allow_loopback))
    {
        return Err("the name resolves to a non-public address".into());
    }
    Ok(Box::new(addresses.into_iter()))
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    use super::*;

    fn socket(ip: IpAddr) -> SocketAddr {
        SocketAddr::new(ip, 443)
    }

    fn answering(addresses: Vec<SocketAddr>) -> PublicOnlyResolver {
        PublicOnlyResolver::with_lookup(
            move |_| {
                let addresses = addresses.clone();
                Box::pin(async move { Ok(addresses) })
            },
            false,
        )
    }

    async fn resolve_to(resolver: &PublicOnlyResolver) -> Result<Vec<SocketAddr>, ()> {
        let name: Name = "mcp.example.test".parse().expect("valid name");
        match resolver.resolve(name).await {
            Ok(addrs) => Ok(addrs.collect()),
            Err(_) => Err(()),
        }
    }

    #[tokio::test]
    async fn public_only_answer_is_returned_unchanged() {
        let addresses = vec![
            socket(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34))),
            socket(IpAddr::V6(Ipv6Addr::new(
                0x2606, 0x2800, 0x220, 1, 0, 0, 0, 0x46,
            ))),
        ];
        let resolver = answering(addresses.clone());
        assert_eq!(resolve_to(&resolver).await, Ok(addresses));
    }

    #[tokio::test]
    async fn one_non_public_address_refuses_the_whole_name() {
        let public = socket(IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)));
        let refused = [
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)),
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
            IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
            IpAddr::V6("fd00::1".parse().expect("literal")),
            IpAddr::V6("64:ff9b::a00:5".parse().expect("literal")),
            IpAddr::V6("::ffff:10.0.0.5".parse().expect("literal")),
        ];
        for ip in refused {
            let resolver = answering(vec![public, socket(ip)]);
            assert_eq!(resolve_to(&resolver).await, Err(()), "{ip} must refuse");
        }
    }

    #[tokio::test]
    async fn loopback_is_refused_unless_allowed() {
        let loopback = [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
        ];
        for ip in loopback {
            let refused = answering(vec![socket(ip)]);
            assert_eq!(resolve_to(&refused).await, Err(()), "{ip} must refuse");

            let allowed = PublicOnlyResolver::with_lookup(
                move |_| Box::pin(async move { Ok(vec![socket(ip)]) }),
                true,
            );
            assert_eq!(resolve_to(&allowed).await, Ok(vec![socket(ip)]), "{ip}");
        }
    }

    #[tokio::test]
    async fn failed_or_empty_lookup_is_refused() {
        let failing = PublicOnlyResolver::with_lookup(
            |_| Box::pin(async { Err(io::Error::other("lookup failed")) }),
            false,
        );
        assert_eq!(resolve_to(&failing).await, Err(()));

        let empty = answering(Vec::new());
        assert_eq!(resolve_to(&empty).await, Err(()));
    }
}
