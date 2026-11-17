use xfalcon_contracts::auth::IdentityProjection;

const MAX_ACCOUNT_ID_BYTES: usize = 160;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct InvalidAccountIdentity;

/// Canonical namespace used by every account-bound local resource.
///
/// Components deliberately exclude the `:` separator, making the tuple
/// encoding unambiguous while retaining compatibility with the existing
/// `tenant:user` on-disk and keyring namespace.
pub(crate) fn canonical_account_id(
    identity: &IdentityProjection,
) -> Result<String, InvalidAccountIdentity> {
    if !is_valid_component(&identity.tenant_id) || !is_valid_component(&identity.user_id) {
        return Err(InvalidAccountIdentity);
    }
    let account_id = format!("{}:{}", identity.tenant_id, identity.user_id);
    if account_id.len() > MAX_ACCOUNT_ID_BYTES {
        return Err(InvalidAccountIdentity);
    }
    Ok(account_id)
}

fn is_valid_component(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}
