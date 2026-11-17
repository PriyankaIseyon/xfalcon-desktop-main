use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::{ContractVersion, CONTRACT_VERSION};

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct IdentityProjection {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub user_id: String,
    pub tenant_id: String,
    pub display_name: String,
    pub email: Option<String>,
}

impl IdentityProjection {
    pub fn new(
        user_id: impl Into<String>,
        tenant_id: impl Into<String>,
        display_name: impl Into<String>,
        email: Option<String>,
    ) -> Self {
        Self {
            version: CONTRACT_VERSION,
            user_id: user_id.into(),
            tenant_id: tenant_id.into(),
            display_name: display_name.into(),
            email,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "status", rename_all = "snake_case")]
#[ts(tag = "status", rename_all = "snake_case")]
pub enum SessionProjection {
    Unauthenticated {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
    },
    Authenticating {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
    },
    Authenticated {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        identity: IdentityProjection,
        #[ts(type = "number")]
        expires_at_ms: i64,
    },
}

impl SessionProjection {
    pub const fn unauthenticated() -> Self {
        Self::Unauthenticated {
            version: CONTRACT_VERSION,
        }
    }

    pub const fn authenticating() -> Self {
        Self::Authenticating {
            version: CONTRACT_VERSION,
        }
    }

    pub fn authenticated(identity: IdentityProjection, expires_at_ms: i64) -> Self {
        Self::Authenticated {
            version: CONTRACT_VERSION,
            identity,
            expires_at_ms,
        }
    }
}

/// Why a sign-in attempt did not produce a session.
///
/// A code, never the service's own message. Those messages name the tenant's
/// issuer, the redirect URI the flow built, and the loopback port it tried —
/// they are written for the operator reading this process's log, which is
/// where they stay. The surface writes copy a person can act on from the code.
///
/// One variant per way the flow can end, so the mapping from the service's
/// error is exhaustive: a new failure mode cannot arrive at the surface as
/// something already on screen.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum AuthLoginErrorCode {
    /// A sign-in is already waiting for the browser. Cancelling it is the way
    /// out; a second attempt cannot borrow the first one's loopback port.
    AlreadyAuthenticating,
    /// This build carries no tenant coordinates.
    NotConfigured,
    /// The default browser could not be opened.
    BrowserUnavailable,
    /// The tenant's authorization metadata could not be fetched.
    DiscoveryFailed,
    /// The tenant names an authorization server this build was not built for.
    IssuerMismatch,
    /// The metadata was fetched but is not usable.
    DiscoveryMetadataInvalid,
    /// The loopback port the callback needs is taken on this device.
    LoopbackUnavailable,
    /// The browser came back, with something that did not verify.
    CallbackInvalid,
    /// Nothing came back from the browser before the deadline.
    CallbackTimedOut,
    /// The authorization server does not know this build's redirect URI.
    RedirectNotRegistered,
    /// The authorization code could not be exchanged for tokens.
    TokenExchangeFailed,
    /// Tokens arrived, and the identity behind them could not be verified.
    IdentityFailed,
    /// This device's clock is far enough from the tenant's that a freshly
    /// minted token reads as not yet valid. Its own code because the remedy is
    /// on the device: an administrator cannot fix a wrong clock here.
    DeviceClockSkew,
    /// The person cancelled.
    Cancelled,
    /// Signed in, but the identity could not be bound to this device's
    /// account-scoped storage — so the session was dropped rather than kept
    /// without somewhere to put what it writes.
    AccountBindFailed,
}

/// A refused sign-in, as the surface receives it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(deny_unknown_fields)]
pub struct AuthLoginError {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub code: AuthLoginErrorCode,
}

impl AuthLoginError {
    #[must_use]
    pub const fn new(code: AuthLoginErrorCode) -> Self {
        Self {
            version: CONTRACT_VERSION,
            code,
        }
    }
}
