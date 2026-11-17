use std::fmt;

use xfalcon_contracts::credentials::{ModelCapabilitiesView, ModelView};

use super::{
    generated,
    mapping::{map_provider, DaBoundaryError},
};

pub(crate) struct ModelRecord {
    model_id: String,
    provider: String,
    display_name: String,
    is_chat: bool,
    is_enabled: bool,
    is_active: bool,
    supports_streaming: bool,
    supports_tools: bool,
    supports_vision: bool,
}

impl ModelRecord {
    pub(crate) fn from_generated(raw: generated::ModelEntity) -> Self {
        Self {
            model_id: raw.model_id,
            provider: raw.provider,
            display_name: raw.display_name,
            is_chat: raw.is_chat,
            is_enabled: raw.is_enabled,
            is_active: raw.is_active,
            supports_streaming: raw.supports_streaming,
            supports_tools: raw.supports_tools,
            supports_vision: raw.supports_vision,
        }
    }
}

impl fmt::Debug for ModelRecord {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModelRecord")
            .field("model_id", &self.model_id)
            .field("provider", &self.provider)
            .field("display_name", &self.display_name)
            .field("is_chat", &self.is_chat)
            .field("is_enabled", &self.is_enabled)
            .field("is_active", &self.is_active)
            .finish()
    }
}

pub(crate) fn project_model(raw: &ModelRecord) -> Result<ModelView, DaBoundaryError> {
    if !raw.is_chat || !raw.is_enabled || !raw.is_active || !raw.supports_streaming {
        return Err(DaBoundaryError::UnavailableModel);
    }
    let provider = map_provider(&raw.provider)?;
    if raw.model_id.trim().is_empty() || raw.display_name.trim().is_empty() {
        return Err(DaBoundaryError::UnavailableModel);
    }

    Ok(ModelView::new(
        raw.model_id.trim(),
        provider,
        raw.display_name.trim(),
        ModelCapabilitiesView::new(
            raw.supports_tools,
            raw.supports_vision,
            false, // DA does not identify a provider-designated user-safe summary.
        ),
    ))
}

pub(crate) fn project_models(
    rows: impl IntoIterator<Item = ModelRecord>,
) -> Result<Vec<ModelView>, DaBoundaryError> {
    rows.into_iter().map(|row| project_model(&row)).collect()
}
