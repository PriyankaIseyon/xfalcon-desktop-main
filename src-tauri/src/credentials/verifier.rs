use async_trait::async_trait;
use xfalcon_contracts::credentials::ProviderId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VerifiedCredential;

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum VerificationError {
    #[error("the provider rejected this credential")]
    InvalidCredential,
    #[error("the credential does not have the required provider permissions")]
    InsufficientPermissions,
    #[error("the provider is temporarily unavailable")]
    ProviderUnavailable,
    #[error("the provider rate limited credential verification")]
    RateLimited,
}

#[async_trait]
pub trait CredentialVerifier: Send + Sync + 'static {
    async fn verify(
        &self,
        provider: ProviderId,
        secret: &str,
    ) -> Result<VerifiedCredential, VerificationError>;
}

pub struct UnavailableCredentialVerifier;

#[async_trait]
impl CredentialVerifier for UnavailableCredentialVerifier {
    async fn verify(
        &self,
        _provider: ProviderId,
        _secret: &str,
    ) -> Result<VerifiedCredential, VerificationError> {
        Err(VerificationError::ProviderUnavailable)
    }
}
