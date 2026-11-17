use schemars::JsonSchema;
use serde::{de::Error as _, Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

pub mod app;
pub mod auth;
pub mod chat;
pub mod credentials;
pub mod custom_models;
pub mod datasets;
pub mod diagnostics;
pub mod export;
pub mod inference;
pub mod mcp;
pub mod plan_providers;
pub mod projects;
pub mod reports;
pub mod settings;
pub mod skills;
pub mod tools;
pub mod update;
pub mod uploads;
pub mod workspace;

#[derive(Clone, Copy, Debug, Default, Eq, Hash, JsonSchema, PartialEq, TS)]
#[schemars(with = "u16", extend("const" = 1))]
#[ts(type = "1")]
pub struct ContractVersion;

impl ContractVersion {
    pub const V1: Self = Self;
}

impl Serialize for ContractVersion {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u16(1)
    }
}

impl<'de> Deserialize<'de> for ContractVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let version = u16::deserialize(deserializer)?;
        if version == 1 {
            Ok(Self::V1)
        } else {
            Err(D::Error::custom(format!(
                "unsupported contract version {version}; expected 1"
            )))
        }
    }
}

pub const CONTRACT_VERSION: ContractVersion = ContractVersion::V1;

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum AppErrorCode {
    AuthRequired,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
pub struct AppError {
    #[schemars(with = "u16", extend("const" = 1))]
    #[ts(type = "1")]
    pub version: ContractVersion,
    pub code: AppErrorCode,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct RunId(String);

impl RunId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for RunId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for RunId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, JsonSchema, PartialEq, Serialize, TS)]
pub struct ConversationId(String);

impl ConversationId {
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for ConversationId {
    fn from(value: &str) -> Self {
        Self(value.to_owned())
    }
}

impl From<String> for ConversationId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(rename_all = "snake_case")]
pub enum RunState {
    Draft,
    Queued,
    Running,
    AwaitingApproval,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    Recovering,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    StateChanged {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        run_id: RunId,
        state: RunState,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize, TS)]
#[serde(tag = "type", rename_all = "snake_case")]
#[ts(tag = "type", rename_all = "snake_case")]
pub enum RuntimeEvent {
    PersistenceRecovered {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        run_id: RunId,
        replayed_events: u32,
    },
    AuditQueueCapacityReached {
        #[schemars(with = "u16", extend("const" = 1))]
        #[ts(type = "1")]
        version: ContractVersion,
        maximum_events: u32,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn schema_has_literal_version(schema: &Value) -> bool {
        match schema {
            Value::Object(object) => {
                let has_literal = object
                    .get("properties")
                    .and_then(Value::as_object)
                    .and_then(|properties| properties.get("version"))
                    .and_then(Value::as_object)
                    .and_then(|version| version.get("const"))
                    == Some(&json!(1));
                has_literal || object.values().any(schema_has_literal_version)
            }
            Value::Array(values) => values.iter().any(schema_has_literal_version),
            _ => false,
        }
    }

    #[test]
    fn error_contract_has_stable_versioned_wire_shape() {
        let error = AppError {
            version: CONTRACT_VERSION,
            code: AppErrorCode::AuthRequired,
            message: "Sign in required".to_owned(),
        };

        let wire = serde_json::to_value(&error).expect("AppError should serialize");
        assert_eq!(
            wire,
            json!({
                "version": CONTRACT_VERSION,
                "code": "auth_required",
                "message": "Sign in required",
            })
        );

        let decoded: AppError = serde_json::from_value(wire).expect("AppError should deserialize");
        assert_eq!(decoded, error);
        let code_wire = serde_json::to_value(AppErrorCode::AuthRequired)
            .expect("AppErrorCode should serialize");
        assert_eq!(code_wire, json!("auth_required"));
        assert_eq!(
            serde_json::from_value::<AppErrorCode>(code_wire)
                .expect("AppErrorCode should deserialize"),
            AppErrorCode::AuthRequired
        );
    }

    #[test]
    fn error_contract_rejects_unsupported_versions() {
        let wire = json!({
            "version": 2,
            "code": "auth_required",
            "message": "Sign in required",
        });

        let error = serde_json::from_value::<AppError>(wire)
            .expect_err("AppError version 2 must be rejected");
        assert!(error.to_string().contains("unsupported contract version 2"));
    }

    #[test]
    fn identifier_newtypes_round_trip_as_wire_strings() {
        let run_id = RunId::from("run-1");
        let conversation_id = ConversationId::from("conversation-1");

        let run_wire = serde_json::to_value(&run_id).expect("RunId should serialize");
        let conversation_wire =
            serde_json::to_value(&conversation_id).expect("ConversationId should serialize");
        assert_eq!(run_wire, json!("run-1"));
        assert_eq!(conversation_wire, json!("conversation-1"));
        assert_eq!(
            serde_json::from_value::<RunId>(run_wire).expect("RunId should deserialize"),
            run_id
        );
        assert_eq!(
            serde_json::from_value::<ConversationId>(conversation_wire)
                .expect("ConversationId should deserialize"),
            conversation_id
        );
    }

    #[test]
    fn run_event_contract_has_stable_versioned_wire_shape() {
        let event = RunEvent::StateChanged {
            version: CONTRACT_VERSION,
            run_id: RunId::from("run-1"),
            state: RunState::AwaitingApproval,
        };

        let wire = serde_json::to_value(&event).expect("RunEvent should serialize");
        assert_eq!(
            wire,
            json!({
                "type": "state_changed",
                "version": CONTRACT_VERSION,
                "run_id": "run-1",
                "state": "awaiting_approval",
            })
        );
        let state_wire =
            serde_json::to_value(RunState::AwaitingApproval).expect("RunState should serialize");
        assert_eq!(state_wire, json!("awaiting_approval"));
        assert_eq!(
            serde_json::from_value::<RunState>(state_wire).expect("RunState should deserialize"),
            RunState::AwaitingApproval
        );
        assert_eq!(
            serde_json::from_value::<RunEvent>(wire).expect("RunEvent should deserialize"),
            event
        );
    }

    #[test]
    fn run_event_contract_rejects_unsupported_versions() {
        let wire = json!({
            "type": "state_changed",
            "version": 2,
            "run_id": "run-1",
            "state": "running",
        });

        let error = serde_json::from_value::<RunEvent>(wire)
            .expect_err("RunEvent version 2 must be rejected");
        assert!(error.to_string().contains("unsupported contract version 2"));
    }

    #[test]
    fn runtime_event_contract_has_stable_versioned_wire_shape() {
        let event = RuntimeEvent::AuditQueueCapacityReached {
            version: CONTRACT_VERSION,
            maximum_events: 256,
        };
        let wire = serde_json::to_value(&event).expect("RuntimeEvent should serialize");
        assert_eq!(
            wire,
            json!({
                "type": "audit_queue_capacity_reached",
                "version": CONTRACT_VERSION,
                "maximum_events": 256,
            })
        );
        assert_eq!(
            serde_json::from_value::<RuntimeEvent>(wire).expect("RuntimeEvent should deserialize"),
            event
        );
        assert!(RuntimeEvent::decl().contains("version: 1"));
    }

    #[test]
    fn generated_contracts_express_version_one_as_a_literal() {
        let app_error_ts = AppError::decl();
        let run_event_ts = RunEvent::decl();
        assert!(
            app_error_ts.contains("version: 1"),
            "unexpected AppError TypeScript: {app_error_ts}"
        );
        assert!(
            run_event_ts.contains("version: 1"),
            "unexpected RunEvent TypeScript: {run_event_ts}"
        );

        let app_error_schema =
            serde_json::to_value(schemars::schema_for!(AppError)).expect("AppError schema");
        let run_event_schema =
            serde_json::to_value(schemars::schema_for!(RunEvent)).expect("RunEvent schema");
        assert!(schema_has_literal_version(&app_error_schema));
        assert!(schema_has_literal_version(&run_event_schema));
    }
}
