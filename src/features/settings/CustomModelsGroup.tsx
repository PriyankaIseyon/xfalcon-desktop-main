import React from "react";
import { Pencil, Plus } from "lucide-react";
import { MODEL_CONTEXT_RESERVED_TOKENS, MODEL_MAX_OUTPUT_TOKENS_PER_TURN } from "../../lib/generated/contracts";
import type {
  CustomModelCreateInput,
  CustomModelDiscoveryResult,
  CustomModelUpdateInput,
  CustomModelView,
  DiscoveredCustomModel,
} from "../../lib/generated/contracts";
import { Alert } from "../../ui/Alert";
import { AlertDialog } from "../../ui/AlertDialog";
import { Badge } from "../../ui/Badge";
import { EmptyState } from "../../ui/EmptyState";
import { Field, Input } from "../../ui/Field";
import { Modal } from "../../ui/Modal";
import { Select } from "../../ui/Select";
import { Spinner } from "../../ui/Spinner";
import { Switch } from "../../ui/Switch";
import { toast } from "../../ui/toastStore";
import { getAuthAccountGeneration } from "../auth/authStore";
import { useAuthSession } from "../auth/useAuth";
import { chatErrorCode, describeChatError } from "../chat/chatPresentation";
import {
  createCustomModel,
  deleteCustomModel,
  discoverCustomModels,
  listCustomModels,
  updateCustomModel,
} from "../models/modelNative";
import { SettingsButton, SettingsRemoveButton } from "./SettingsControls";
import { SettingsActions, SettingsGroup, SettingsGroupHeader } from "./SettingsLayout";

type ListState =
  { status: "loading" } | { status: "ready"; models: CustomModelView[] } | { status: "error"; message: string };

const SAFE_CUSTOM_MODEL_FAILURES = new Set([
  "Private models are disabled on this deployment.",
  "The private model limit was reached, or this model can no longer be changed.",
  "This private model was removed. Add it again if you still need it.",
  "Check the private model fields and try again.",
  "Enter a valid API base URL, such as http://localhost:8000/v1.",
  "Enter an API base URL.",
  "The provider rejected this API key.",
  "Enter a valid API key.",
  "The provider could not use this model configuration. Check the model ID, API base URL, and model settings.",
  "Could not reach the provider. Check the API base URL and network connection.",
  "The provider connection test timed out. Check the API base URL and try again.",
  "The provider could not complete the connection test. Check the API key, model ID, and provider account.",
  "Enter the API key again to test this private model.",
  "Enter a new API key when changing the endpoint URL.",
  "Could not load this private model's saved API key. Try again.",
  "Could not load this private model's settings. Try again.",
  "Could not check for existing private models. Try again.",
  "This model is already added. Review your private models.",
  "Sign in again to test this private model.",
  "sampling.temperature must be between 0 and 2.",
  "sampling.top_p must be between 0 and 1.",
]);

const CUSTOM_MODEL_VALIDATION_FIELDS = [
  "base_url",
  "upstream_model_id",
  "secret",
  "display_name",
  "context_length",
  "max_output_tokens",
  "supports_tools",
  "supports_vision",
  "is_enabled",
  "is_active",
] as const;

/**
 * Custom-model 422 reasons cross IPC as `ChatOperationError.message`. The
 * native boundary already filters them, but this surface repeats the same
 * allowlist before rendering. A malformed or future error therefore cannot
 * echo a submitted key or full endpoint into Settings.
 */
function describeCustomModelError(cause: unknown, fallback: string, sensitiveValues: readonly string[] = []): string {
  const code = chatErrorCode(cause);
  if (code === null) return fallback;
  const message = describeChatError(cause, fallback);
  if (code !== "failed") return message;
  if (
    message ===
    `context_length must leave room for model output and ${MODEL_CONTEXT_RESERVED_TOKENS} reserved tokens for xFalcon's instructions and tools.`
  ) {
    return "These limits leave no room for input. Lower the output limit or check your model's context length, then try again.";
  }

  const containsSensitiveValue = sensitiveValues.some(
    (value) => value.trim().length > 0 && message.includes(value.trim()),
  );
  const safeValidationReason =
    message.length <= 256 &&
    !message.includes("://") &&
    !containsSensitiveValue &&
    ![...message].some((character) => {
      const codePoint = character.codePointAt(0) ?? 0;
      return codePoint <= 31 || codePoint === 127;
    }) &&
    CUSTOM_MODEL_VALIDATION_FIELDS.some((field) => message.startsWith(field));

  const safeDuplicateReason =
    !containsSensitiveValue &&
    message.length <= 256 &&
    message.startsWith("This model is already added as “") &&
    message.endsWith("”.") &&
    ![...message].some((character) => (character.codePointAt(0) ?? 0) <= 31 || character === "\u007f");
  return SAFE_CUSTOM_MODEL_FAILURES.has(message) || safeValidationReason || safeDuplicateReason ? message : fallback;
}

function mutationMayHaveCommitted(cause: unknown): boolean {
  const code = chatErrorCode(cause);
  return (
    code === "temporarily_unavailable" ||
    code === "service_unavailable" ||
    (code === "failed" && describeChatError(cause, "") === "The model catalog returned an unexpected response.")
  );
}

export interface CustomModelsGroupProps {
  onCatalogChanged: () => void;
}

export function CustomModelsGroup(props: CustomModelsGroupProps) {
  const session = useAuthSession();
  const accountKey =
    session.status === "authenticated" ? `${session.identity.tenant_id}:${session.identity.user_id}` : "signed-out";

  // Re-mounting the account-owned state prevents even one paint of the prior
  // account's model names or endpoint configuration after an account switch.
  return <CustomModelsAccount key={accountKey} {...props} />;
}

function CustomModelsAccount({ onCatalogChanged }: CustomModelsGroupProps) {
  const [state, setState] = React.useState<ListState>({ status: "loading" });
  const [reloadToken, setReloadToken] = React.useState(0);
  const [saveNotice, setSaveNotice] = React.useState<string | null>(null);
  const [editor, setEditor] = React.useState<{ open: boolean; model: CustomModelView | null }>({
    open: false,
    model: null,
  });
  const [pendingRemoval, setPendingRemoval] = React.useState<CustomModelView | null>(null);
  const [removing, setRemoving] = React.useState(false);
  const [removeError, setRemoveError] = React.useState<string | null>(null);

  React.useEffect(() => {
    let current = true;
    const accountGeneration = getAuthAccountGeneration();
    setState({ status: "loading" });
    void listCustomModels().then(
      (models) => {
        if (current && accountGeneration === getAuthAccountGeneration()) {
          setState({ status: "ready", models });
        }
      },
      (cause: unknown) => {
        if (current && accountGeneration === getAuthAccountGeneration()) {
          setState({
            status: "error",
            message: describeCustomModelError(cause, "Private models could not be loaded."),
          });
        }
      },
    );
    return () => {
      current = false;
    };
  }, [reloadToken]);

  const reload = React.useCallback(() => {
    setReloadToken((token) => token + 1);
  }, []);

  const changed = React.useCallback(() => {
    reload();
    onCatalogChanged();
  }, [onCatalogChanged, reload]);

  React.useEffect(() => {
    if (
      state.status === "ready" &&
      pendingRemoval &&
      !removing &&
      !state.models.some((model) => model.id === pendingRemoval.id)
    ) {
      setPendingRemoval(null);
      setRemoveError(null);
      setSaveNotice("This private model is no longer in the list.");
      onCatalogChanged();
    }
  }, [onCatalogChanged, pendingRemoval, removing, state]);

  const remove = async (model: CustomModelView) => {
    const accountGeneration = getAuthAccountGeneration();
    setRemoving(true);
    setRemoveError(null);
    try {
      await deleteCustomModel(model.id);
      if (accountGeneration !== getAuthAccountGeneration()) return;
      setPendingRemoval(null);
      setSaveNotice(null);
      toast.success(`Removed the private model “${model.display_name}”.`);
      changed();
    } catch (cause: unknown) {
      if (accountGeneration !== getAuthAccountGeneration()) return;
      if (
        mutationMayHaveCommitted(cause) ||
        describeChatError(cause, "") === "This private model was removed. Add it again if you still need it."
      ) {
        setPendingRemoval(null);
        setSaveNotice("The removal could not be confirmed. Review the updated private model list.");
        changed();
        return;
      }
      setRemoveError(describeCustomModelError(cause, "The private model could not be removed. Try again."));
      reload();
    } finally {
      if (accountGeneration === getAuthAccountGeneration()) setRemoving(false);
    }
  };

  return (
    <SettingsGroup id="settings-private-models" aria-labelledby="settings-private-models-heading">
      <SettingsGroupHeader
        headingId="settings-private-models-heading"
        title="Private models"
        description="Connect a private OpenAI-compatible model with your API key. Stored keys stay hidden."
      >
        <SettingsButton
          icon={<Plus />}
          onClick={() => {
            setSaveNotice(null);
            setEditor({ open: true, model: null });
          }}
        >
          Add private model
        </SettingsButton>
      </SettingsGroupHeader>

      {saveNotice ? (
        <Alert title="Review private models" tone="warn">
          {saveNotice}
        </Alert>
      ) : null}

      {state.status === "loading" ? (
        <div className="settings-group__state text-secondary">
          <Spinner decorative size={16} />
          <span>Loading private models…</span>
        </div>
      ) : null}

      {state.status === "error" ? (
        <Alert className="settings-group__state" title="Private models unavailable" tone="danger">
          <p>{state.message}</p>
          <SettingsButton className="mt-2" onClick={reload}>
            Try again
          </SettingsButton>
        </Alert>
      ) : null}

      {state.status === "ready" && state.models.length === 0 ? (
        <EmptyState
          className="settings-group__empty"
          description="Add an API base URL and key, then choose a model or enter its exact name."
          eyebrow="Optional"
          title={<p className="type-body font-semibold text-foreground">No private models</p>}
        />
      ) : null}

      {state.status === "ready" && state.models.length > 0 ? (
        <ul aria-label="Private models">
          {state.models.map((model) => (
            <li className="settings-collection-item settings-collection-item--card" key={model.id}>
              <div className="flex flex-wrap items-start justify-between gap-3">
                <div className="flex min-w-0 flex-col gap-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <strong className="type-body break-words text-foreground">{model.display_name}</strong>
                    <Badge tone="accent">Custom</Badge>
                    {model.max_output_tokens === null || model.max_output_tokens <= 0 ? (
                      <Badge tone="warn">Needs settings</Badge>
                    ) : !model.is_enabled || !model.is_active || !model.supports_tools ? (
                      <Badge tone="neutral">Unavailable</Badge>
                    ) : null}
                  </div>
                  <p className="settings-collection-item__summary break-all">{model.base_url}</p>
                  {model.max_output_tokens === null || model.max_output_tokens <= 0 ? (
                    <p className="type-caption text-muted">Set an output limit to use this model.</p>
                  ) : null}
                </div>
                <SettingsActions>
                  <SettingsButton
                    icon={<Pencil />}
                    onClick={() => {
                      setSaveNotice(null);
                      setEditor({ open: true, model });
                    }}
                  >
                    Edit
                  </SettingsButton>
                  <SettingsRemoveButton
                    onClick={() => {
                      setSaveNotice(null);
                      setRemoveError(null);
                      setPendingRemoval(model);
                    }}
                  />
                </SettingsActions>
              </div>
            </li>
          ))}
        </ul>
      ) : null}

      <CustomModelEditor
        model={editor.model}
        open={editor.open}
        onClose={() => setEditor({ open: false, model: null })}
        onUnconfirmed={() => {
          setEditor({ open: false, model: null });
          setSaveNotice("The save could not be confirmed. Review the updated private model list before trying again.");
          changed();
        }}
        onSaved={(model, created) => {
          setEditor({ open: false, model: null });
          setSaveNotice(null);
          toast.success(`${created ? "Added" : "Updated"} the private model “${model.display_name}”.`);
          changed();
        }}
      />

      <AlertDialog
        busy={removing}
        confirmLabel="Remove model"
        description="This removes the model and its stored API key from your account. Existing chats that use it can no longer continue."
        destructive
        error={removeError}
        open={pendingRemoval !== null}
        onCancel={() => {
          setPendingRemoval(null);
          setRemoveError(null);
        }}
        onConfirm={() => {
          if (pendingRemoval) void remove(pendingRemoval);
        }}
        title={pendingRemoval ? `Remove “${pendingRemoval.display_name}”?` : "Remove private model?"}
      />
    </SettingsGroup>
  );
}

interface FormValues {
  baseUrl: string;
  secret: string;
  upstreamModelId: string;
  displayName: string;
  contextLength: string;
  maxOutputTokens: string;
  temperature: string;
  topP: string;
  supportsTools: boolean;
  supportsVision: boolean;
  available: boolean;
}

const SUGGESTED_MODEL_VALUES = {
  upstreamModelId: "",
  displayName: "",
  contextLength: "32768",
  maxOutputTokens: "4096",
  temperature: "0.7",
  topP: "0.9",
  supportsTools: false,
  supportsVision: false,
};
const EMPTY_FORM: FormValues = {
  baseUrl: "https://llm-gateway.iseyon.workers.dev/v1",
  secret: "",
  ...SUGGESTED_MODEL_VALUES,
  available: true,
};

function valuesFor(model: CustomModelView | null): FormValues {
  if (!model) return EMPTY_FORM;
  return {
    baseUrl: model.base_url,
    secret: "",
    upstreamModelId: model.upstream_model_id,
    displayName: model.display_name,
    contextLength: String(model.context_length),
    maxOutputTokens: model.max_output_tokens === null ? "" : String(model.max_output_tokens),
    temperature: model.sampling.temperature === null ? "" : String(model.sampling.temperature),
    topP: model.sampling.top_p === null ? "" : String(model.sampling.top_p),
    supportsTools: model.supports_tools,
    supportsVision: model.supports_vision,
    available: model.is_enabled && model.is_active,
  };
}

function discoveredValues(model: DiscoveredCustomModel): typeof SUGGESTED_MODEL_VALUES {
  const supported = model.supported_sampling_parameters;
  // Suggestions fit alongside whichever limit the endpoint did report.
  // Reported values are preserved exactly, even if the pair needs correction.
  const output = model.max_output_tokens ?? Number(SUGGESTED_MODEL_VALUES.maxOutputTokens);
  const suggestedInput =
    Number(SUGGESTED_MODEL_VALUES.contextLength) -
    Number(SUGGESTED_MODEL_VALUES.maxOutputTokens) -
    MODEL_CONTEXT_RESERVED_TOKENS;
  const context =
    model.context_length ??
    Math.min(
      2_147_483_647,
      Math.max(
        Number(SUGGESTED_MODEL_VALUES.contextLength),
        output + 1,
        Math.min(output, MODEL_MAX_OUTPUT_TOKENS_PER_TURN) + MODEL_CONTEXT_RESERVED_TOKENS + suggestedInput,
      ),
    );
  const suggestedOutput = Math.min(
    Number(SUGGESTED_MODEL_VALUES.maxOutputTokens),
    Math.max(1, Math.floor((context - MODEL_CONTEXT_RESERVED_TOKENS) / 2)),
  );
  return {
    upstreamModelId: model.upstream_model_id,
    displayName: [...model.display_name].slice(0, 128).join(""),
    contextLength: String(context),
    maxOutputTokens: String(model.max_output_tokens ?? suggestedOutput),
    temperature:
      supported !== null && !supported.includes("temperature")
        ? ""
        : String(model.sampling.temperature ?? SUGGESTED_MODEL_VALUES.temperature),
    topP:
      supported !== null && !supported.includes("top_p")
        ? ""
        : String(model.sampling.top_p ?? SUGGESTED_MODEL_VALUES.topP),
    supportsTools: model.supports_tools ?? false,
    supportsVision: model.supports_vision ?? false,
  };
}

const DISCOVERY_NOTICES = {
  unsupported: "This endpoint has no model list. Enter the exact model ID and check the suggested settings.",
  unreachable:
    "Could not reach the model list. Go back to check the connection, or enter the model ID and review the suggested settings.",
  invalid_response:
    "This endpoint did not return a usable model list. Enter the exact model ID and check the suggested settings.",
  empty: "The endpoint returned no models for this key. Enter a model name if your provider has given you one.",
} as const;

type ValidatedField =
  "baseUrl" | "upstreamModelId" | "displayName" | "contextLength" | "maxOutputTokens" | "temperature" | "topP";

/** A `.` or `..` path segment, also spelled with `%2e`: the native code refuses these. */
function hasDotPathSegment(value: string): boolean {
  const path = value.replace(/^[a-z][a-z0-9+.-]*:\/\/[^/]*/i, "");
  return path.split("/").some((segment) => {
    const decoded = segment.toLowerCase().replaceAll("%2e", ".");
    return decoded === "." || decoded === "..";
  });
}

function fieldError(values: FormValues, field: ValidatedField): string | undefined {
  const value = values[field].trim();
  if (field === "baseUrl") {
    // Checked on the raw text first: `new URL` would strip tabs and newlines, turn backslashes into slashes
    // and punycode non-ASCII hosts, so the form would accept what the native code refuses.
    const invalid =
      "Enter an API base URL that starts with http:// or https://, without credentials, a query, or a fragment.";
    if (!/^[\x21-\x7e]{1,2048}$/.test(value) || /[\\?#]/.test(value)) {
      return invalid;
    }
    try {
      const url = new URL(value);
      // `new URL` rewrites numeric IPv4 spellings such as 127.1 or 2130706433 to another address, so the
      // typed host must already be the dotted-decimal address the URL parser produces.
      const typedHost = /^[a-z][a-z0-9+.-]*:\/\/([^/:?#]*)/i.exec(value)?.[1].toLowerCase();
      const rewrittenIpv4 = /^\d{1,3}(\.\d{1,3}){3}$/.test(url.hostname) && typedHost !== url.hostname;
      if (
        (url.protocol === "http:" || url.protocol === "https:") &&
        !url.username &&
        !url.password &&
        url.port !== "0" &&
        !hasDotPathSegment(value) &&
        !rewrittenIpv4
      ) {
        return;
      }
    } catch {
      // The native transport repeats full URL validation.
    }
    return invalid;
  }
  if (field === "displayName" || field === "upstreamModelId") {
    const limit = field === "displayName" ? 128 : 256;
    const visible = ![...value].some((character) => {
      const codePoint = character.codePointAt(0) ?? 0;
      return codePoint <= 31 || codePoint === 127;
    });
    if (value && [...value].length <= limit && visible) return;
    return `Enter ${field === "displayName" ? "a display name" : "the exact model name"} of ${limit} characters or fewer.`;
  }
  if (field === "temperature" || field === "topP") {
    if (!value) return;
    const limit = field === "temperature" ? 2 : 1;
    const number = Number(value);
    if (Number.isFinite(number) && number >= 0 && number <= limit) return;
    return `Enter a number from 0 to ${limit}, or leave blank for the provider default.`;
  }
  const number = Number(value);
  if (!value || !Number.isInteger(number) || number < 1 || number > 2_147_483_647) {
    return "Enter a whole number from 1 to 2,147,483,647 tokens.";
  }
  if (field === "contextLength" && number <= MODEL_CONTEXT_RESERVED_TOKENS + 1) {
    return `This context is too small for xFalcon. Use a model with at least ${(MODEL_CONTEXT_RESERVED_TOKENS + 2).toLocaleString("en-US")} context tokens.`;
  }
  if (field === "maxOutputTokens" && number >= Number(values.contextLength)) {
    return "Maximum output tokens must be smaller than the context length, leaving room for the prompt.";
  }
  const context = Number(values.contextLength);
  if (
    field === "maxOutputTokens" &&
    Number.isInteger(context) &&
    context > MODEL_CONTEXT_RESERVED_TOKENS + 1 &&
    context <= 2_147_483_647 &&
    context - Math.min(number, MODEL_MAX_OUTPUT_TOKENS_PER_TURN) <= MODEL_CONTEXT_RESERVED_TOKENS
  ) {
    return `Use ${(context - MODEL_CONTEXT_RESERVED_TOKENS - 1).toLocaleString("en-US")} output tokens or fewer to leave room for input and xFalcon's instructions and tools.`;
  }
  return undefined;
}

interface CustomModelEditorProps {
  open: boolean;
  model: CustomModelView | null;
  onClose: () => void;
  onUnconfirmed: () => void;
  onSaved: (model: CustomModelView, created: boolean) => void;
}

function CustomModelEditor({ open, model, onClose, onUnconfirmed, onSaved }: CustomModelEditorProps) {
  const [values, setValues] = React.useState<FormValues>(EMPTY_FORM);
  const [step, setStep] = React.useState<"connection" | "model">("connection");
  const [discovery, setDiscovery] = React.useState<CustomModelDiscoveryResult | null>(null);
  const [discoveryKey, setDiscoveryKey] = React.useState("");
  const [manual, setManual] = React.useState(false);
  const [search, setSearch] = React.useState("");
  const [touched, setTouched] = React.useState<Partial<Record<ValidatedField, boolean>>>({});
  const [discovering, setDiscovering] = React.useState(false);
  const [submitting, setSubmitting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const submittingRef = React.useRef(false);
  const discoveringRef = React.useRef(false);
  const requestEpoch = React.useRef(0);
  const errorRef = React.useRef<HTMLDivElement>(null);
  const modelNameRef = React.useRef<HTMLInputElement>(null);
  const modelPickerRef = React.useRef<HTMLDivElement>(null);

  React.useEffect(() => {
    if (open) {
      setValues(valuesFor(model));
      setStep(model ? "model" : "connection");
      setDiscovery(null);
      setDiscoveryKey("");
      setManual(model !== null);
      setSearch("");
      setTouched({ contextLength: true, maxOutputTokens: true });
      setSubmitting(false);
      setDiscovering(false);
      setError(null);
      submittingRef.current = false;
      discoveringRef.current = false;
    }
    return () => {
      requestEpoch.current += 1;
    };
  }, [model, open]);

  React.useEffect(() => {
    if (error) errorRef.current?.focus();
  }, [error]);
  React.useEffect(() => {
    if (open && step === "model" && !model) {
      if (manual) modelNameRef.current?.focus();
      else modelPickerRef.current?.querySelector("button")?.focus();
    }
  }, [manual, model, open, step]);

  const baseUrl = values.baseUrl.trim();
  const secret = values.secret;
  const endpointChanged = model !== null && baseUrl !== model.base_url;
  const enabling = model !== null && values.available && (!model.is_enabled || !model.is_active);
  const keyRequired = model === null || endpointChanged || enabling;
  const busy = submitting || discovering;
  const selected = discovery?.models.find((entry) => entry.upstream_model_id === values.upstreamModelId.trim());
  const samplingSupported = (parameter: string) =>
    selected?.supported_sampling_parameters == null || selected.supported_sampling_parameters.includes(parameter);
  const modelValues: FormValues = {
    ...values,
    temperature: samplingSupported("temperature") ? values.temperature : "",
    topP: samplingSupported("top_p") ? values.topP : "",
  };
  const duplicate =
    discoveryKey === secret
      ? discovery?.existing_models.find((entry) => entry.upstream_model_id === values.upstreamModelId.trim())
      : undefined;
  const canConnect = !busy && !fieldError(values, "baseUrl") && secret.trim().length > 0;
  const validModel = (
    ["baseUrl", "upstreamModelId", "displayName", "contextLength", "maxOutputTokens", "temperature", "topP"] as const
  ).every((field) => !fieldError(modelValues, field));
  const update = model ? buildUpdate(model, modelValues) : null;
  const verifiesConnection =
    model === null ||
    Boolean(
      update?.base_url ||
      update?.secret ||
      update?.upstream_model_id ||
      update?.max_output_tokens ||
      update?.sampling ||
      enabling,
    );
  const canSubmit =
    step === "model" &&
    validModel &&
    (!keyRequired || secret.trim().length > 0) &&
    (!model || (update !== null && Object.keys(update).length > 0)) &&
    !duplicate &&
    !busy;
  const filteredModels = React.useMemo(() => {
    const query = search.trim().toLowerCase();
    const options = (discovery?.models ?? [])
      .filter((entry) => `${entry.upstream_model_id} ${entry.display_name}`.toLowerCase().includes(query))
      .slice(0, 50);
    if (selected && !options.some((entry) => entry.upstream_model_id === selected.upstream_model_id))
      options.unshift(selected);
    return options;
  }, [discovery, search, selected]);

  const changeValues = (change: (current: FormValues) => FormValues) => {
    if (submittingRef.current || discoveringRef.current) return;
    setError(null);
    setValues(change);
  };
  const set = <Key extends keyof FormValues>(key: Key, value: FormValues[Key]) =>
    changeValues((current) => ({ ...current, [key]: value }));
  const touch = (field: ValidatedField) => setTouched((current) => ({ ...current, [field]: true }));
  const validation = (field: ValidatedField) => (touched[field] ? fieldError(modelValues, field) : undefined);
  const close = () => {
    if (submittingRef.current) return;
    requestEpoch.current += 1;
    discoveringRef.current = false;
    setValues(EMPTY_FORM);
    setDiscovery(null);
    setDiscoveryKey("");
    setError(null);
    onClose();
  };

  const discover = async () => {
    if (!canConnect || discoveringRef.current || submittingRef.current) return;
    const accountGeneration = getAuthAccountGeneration();
    const epoch = ++requestEpoch.current;
    const current = () => epoch === requestEpoch.current && accountGeneration === getAuthAccountGeneration();
    discoveringRef.current = true;
    setDiscovering(true);
    setError(null);
    try {
      const result = await discoverCustomModels({ base_url: baseUrl, secret });
      if (!current()) return;
      setDiscovery(result);
      setDiscoveryKey(secret);
      setManual(result.models.length === 0);
      setSearch("");
      setTouched({ contextLength: true, maxOutputTokens: true });
      setValues((previous) => ({
        ...previous,
        ...SUGGESTED_MODEL_VALUES,
        ...(result.models.length === 1 ? discoveredValues(result.models[0]) : {}),
      }));
      setStep("model");
    } catch (cause: unknown) {
      if (!current()) return;
      setValues((previous) => ({ ...previous, secret: "" }));
      setDiscoveryKey("");
      setError(
        describeCustomModelError(cause, "Could not check this endpoint. Check the URL and API key, then try again.", [
          secret,
          baseUrl,
        ]),
      );
    } finally {
      if (current()) {
        discoveringRef.current = false;
        setDiscovering(false);
      }
    }
  };

  const submit = async () => {
    if (!canSubmit || submittingRef.current || discoveringRef.current) return;
    const accountGeneration = getAuthAccountGeneration();
    const epoch = ++requestEpoch.current;
    const current = () => epoch === requestEpoch.current && accountGeneration === getAuthAccountGeneration();
    submittingRef.current = true;
    setSubmitting(true);
    setError(null);
    try {
      const saved = model
        ? await updateCustomModel(model.id, buildUpdate(model, modelValues))
        : await createCustomModel(buildCreate(modelValues));
      if (!current()) return;
      setValues((previous) => ({ ...previous, secret: "" }));
      setDiscoveryKey("");
      onSaved(saved, model === null);
    } catch (cause: unknown) {
      if (!current()) return;
      setValues((previous) => ({ ...previous, secret: "" }));
      setDiscoveryKey("");
      if (mutationMayHaveCommitted(cause)) {
        onUnconfirmed();
        return;
      }
      setError(
        describeCustomModelError(
          cause,
          `The private model could not be ${model ? "updated" : "added"}. Check the settings and try again.`,
          [secret, baseUrl],
        ),
      );
    } finally {
      if (current()) {
        submittingRef.current = false;
        setSubmitting(false);
      }
    }
  };

  const showConnection = step === "connection" || model !== null;
  const showKey = showConnection || !discoveryKey || error !== null || !secret.trim();
  const prefilledValues = selected && !manual ? discoveredValues(selected) : SUGGESTED_MODEL_VALUES;
  const settingSource = (
    field: "contextLength" | "maxOutputTokens" | "temperature" | "topP",
    reported?: number | null,
  ) => {
    if (model || values[field] !== prefilledValues[field]) return "";
    return !manual && reported != null
      ? "Provided by your model endpoint. "
      : "Suggested by xFalcon. Check and update for your model. ";
  };

  return (
    <Modal
      dismissible={!submitting}
      description={
        step === "connection"
          ? "Step 1 of 2 · Connect an API that supports OpenAI Chat Completions."
          : model
            ? "Review the model settings. Leave the key blank to keep it."
            : "Step 2 of 2 · Choose a model and review its settings."
      }
      footer={
        <>
          <SettingsButton disabled={submitting} onClick={close}>
            Cancel
          </SettingsButton>
          {!model && step === "model" ? (
            <SettingsButton
              disabled={busy}
              onClick={() => {
                setStep("connection");
                setDiscovery(null);
                setDiscoveryKey("");
                setError(null);
                setSearch("");
              }}
            >
              Back
            </SettingsButton>
          ) : null}
          <SettingsButton
            disabled={step === "connection" ? !canConnect : !canSubmit}
            loading={busy}
            onClick={() => void (step === "connection" ? discover() : submit())}
            variant="primary"
          >
            {step === "connection"
              ? "Next"
              : submitting
                ? verifiesConnection
                  ? "Testing and saving…"
                  : "Saving…"
                : model
                  ? verifiesConnection
                    ? "Test and save changes"
                    : "Save changes"
                  : "Test and add model"}
          </SettingsButton>
        </>
      }
      onClose={close}
      open={open}
      size="lg"
      title={model ? `Edit “${model.display_name}”` : "Add private model"}
    >
      <div className="grid gap-4 sm:grid-cols-2">
        {showConnection ? (
          <Field
            className="sm:col-span-2"
            label="Endpoint URL"
            hint="Use the API base URL, usually ending in /v1, without /chat/completions. A server on this computer or your network works too, for example http://localhost:11434/v1. An http address sends the key unencrypted. Model discovery uses this URL followed by /models."
            error={validation("baseUrl")}
            required
          >
            <Input
              autoComplete="url"
              disabled={busy}
              onBlur={() => touch("baseUrl")}
              onChange={(event) => set("baseUrl", event.target.value)}
              placeholder="https://api.example.com/v1"
              spellCheck={false}
              maxLength={2048}
              type="url"
              value={values.baseUrl}
            />
          </Field>
        ) : (
          <p className="type-caption break-all text-muted sm:col-span-2">{baseUrl}</p>
        )}

        {showKey ? (
          <Field
            className="sm:col-span-2"
            hint={
              model
                ? endpointChanged
                  ? "A new key is required because the endpoint changed."
                  : enabling
                    ? "Re-enter the key to make this model available again."
                    : "Leave blank to keep the stored key."
                : step === "model"
                  ? "Re-enter your key to retry the save. The model settings have been kept."
                  : "The key is used to check this endpoint. It is encrypted when you save the model and cannot be displayed later."
            }
            label="API key"
            required={keyRequired}
          >
            <Input
              autoComplete="new-password"
              disabled={busy}
              onChange={(event) => set("secret", event.target.value)}
              spellCheck={false}
              type="password"
              value={values.secret}
            />
          </Field>
        ) : null}

        {step === "model" ? (
          <>
            {duplicate ? (
              <Alert className="sm:col-span-2" title="Model already added" tone="warn">
                This model is already added as “{duplicate.display_name}”. Choose another model, or go back to change
                the URL or API key.
              </Alert>
            ) : null}
            {discovery?.unavailable_reason ? (
              <Alert className="sm:col-span-2" title="Enter model details">
                {DISCOVERY_NOTICES[discovery.unavailable_reason]}
              </Alert>
            ) : null}
            {!manual && discovery && discovery.models.length > 0 ? (
              <>
                {discovery.models.length > 20 ? (
                  <Field
                    className="sm:col-span-2"
                    label="Search models"
                    hint="Search by name or ID. Up to 50 matching models are shown."
                  >
                    <Input
                      disabled={busy}
                      value={search}
                      onChange={(event) => setSearch(event.target.value)}
                      type="search"
                    />
                  </Field>
                ) : null}
                <div className="sm:col-span-2" ref={modelPickerRef}>
                  <Field
                    label="Model"
                    required
                    hint="The exact model ID returned by your endpoint is used for requests."
                  >
                    <Select
                      disabled={busy}
                      options={filteredModels.map((entry) => ({
                        value: entry.upstream_model_id,
                        label:
                          entry.display_name === entry.upstream_model_id
                            ? entry.upstream_model_id
                            : `${entry.display_name} · ${entry.upstream_model_id}`,
                      }))}
                      placeholder="Choose a model"
                      value={values.upstreamModelId || undefined}
                      onChange={(id) => {
                        const entry = discovery.models.find((candidate) => candidate.upstream_model_id === id);
                        if (entry) {
                          changeValues((previous) => ({ ...previous, ...discoveredValues(entry) }));
                          setTouched({ contextLength: true, maxOutputTokens: true });
                        }
                      }}
                    />
                  </Field>
                </div>
                <div className="sm:col-span-2">
                  <SettingsButton
                    disabled={busy}
                    onClick={() => {
                      setManual(true);
                      changeValues((previous) => ({ ...previous, ...SUGGESTED_MODEL_VALUES }));
                      setTouched({ contextLength: true, maxOutputTokens: true });
                    }}
                  >
                    Enter a model name instead
                  </SettingsButton>
                </div>
              </>
            ) : (
              <Field
                className="sm:col-span-2"
                label="Model name"
                hint="Use the exact ID from your provider, including any prefix, such as ollama/<model> or openai/<model>. The app does not add a prefix."
                error={validation("upstreamModelId")}
                required
              >
                <Input
                  ref={modelNameRef}
                  disabled={busy}
                  maxLength={256}
                  spellCheck={false}
                  value={values.upstreamModelId}
                  onBlur={() => touch("upstreamModelId")}
                  onChange={(event) => {
                    const id = event.target.value;
                    changeValues((previous) => ({
                      ...previous,
                      upstreamModelId: id,
                      displayName:
                        previous.displayName === previous.upstreamModelId.slice(0, 128)
                          ? id.slice(0, 128)
                          : previous.displayName,
                    }));
                  }}
                />
              </Field>
            )}

            <Field
              className="sm:col-span-2"
              label="Display name"
              hint="A name for this model in your model list."
              error={validation("displayName")}
              required
            >
              <Input
                disabled={busy}
                maxLength={128}
                value={values.displayName}
                onBlur={() => touch("displayName")}
                onChange={(event) => set("displayName", event.target.value)}
              />
            </Field>
            <Field
              label="Context length"
              hint={`${settingSource("contextLength", selected?.context_length)}Total tokens for input and output. Use the server's configured limit.`}
              error={validation("contextLength")}
              required
            >
              <Input
                disabled={busy}
                type="number"
                min={1}
                max={2_147_483_647}
                step={1}
                value={values.contextLength}
                onBlur={() => touch("contextLength")}
                onChange={(event) => set("contextLength", event.target.value)}
              />
            </Field>
            <Field
              label="Maximum output tokens"
              hint={`${settingSource("maxOutputTokens", selected?.max_output_tokens)}Tokens per reply, including reasoning. Leave room for input within the context length.`}
              error={validation("maxOutputTokens")}
              required
            >
              <Input
                disabled={busy}
                type="number"
                min={1}
                max={2_147_483_647}
                step={1}
                value={values.maxOutputTokens}
                onBlur={() => touch("maxOutputTokens")}
                onChange={(event) => set("maxOutputTokens", event.target.value)}
              />
            </Field>

            <p className="type-caption text-muted sm:col-span-2">
              Both token fields accept up to 2,147,483,647. xFalcon requests at most{" "}
              {MODEL_MAX_OUTPUT_TOKENS_PER_TURN.toLocaleString("en-US")} output tokens per reply and reserves{" "}
              {MODEL_CONTEXT_RESERVED_TOKENS.toLocaleString("en-US")} context tokens for instructions and tools.
            </p>

            <div className="sm:col-span-2">
              <Switch
                checked={values.supportsTools}
                disabled={busy}
                label="Supports tool calling"
                onChange={(next) => set("supportsTools", next)}
              />
              <p className="type-caption mt-1 text-muted">
                {selected?.supports_tools != null && !manual
                  ? "Review this pre-filled capability with your provider. "
                  : "Confirm this with your provider. "}
                Tool calling is required to use the model in xFalcon chat.
              </p>
            </div>
            <div className="sm:col-span-2">
              <Switch
                checked={values.supportsVision}
                disabled={busy}
                label="Supports image input"
                onChange={(next) => set("supportsVision", next)}
              />
              <p className="type-caption mt-1 text-muted">
                {selected?.supports_vision != null && !manual
                  ? "Review this pre-filled capability with your provider."
                  : "Enable only if the model and endpoint accept images."}
              </p>
            </div>

            <p className="type-caption text-muted sm:col-span-2">
              Clear Temperature or Top P to use provider defaults. These choices are saved on this device.
            </p>
            <Field
              label="Temperature"
              hint={
                samplingSupported("temperature")
                  ? `${settingSource("temperature", selected?.sampling.temperature)}Optional, from 0 to 2. Lower values reduce variation.`
                  : "The endpoint reports that temperature is unsupported."
              }
              error={validation("temperature")}
            >
              <Input
                disabled={busy || !samplingSupported("temperature")}
                type="number"
                min={0}
                max={2}
                step="any"
                placeholder="Provider default"
                value={modelValues.temperature}
                onBlur={() => touch("temperature")}
                onChange={(event) => set("temperature", event.target.value)}
              />
            </Field>
            <Field
              label="Top P"
              hint={
                samplingSupported("top_p")
                  ? `${settingSource("topP", selected?.sampling.top_p)}Optional, from 0 to 1. Usually adjust this or Temperature.`
                  : "The endpoint reports that Top P is unsupported."
              }
              error={validation("topP")}
            >
              <Input
                disabled={busy || !samplingSupported("top_p")}
                type="number"
                min={0}
                max={1}
                step="any"
                placeholder="Provider default"
                value={modelValues.topP}
                onBlur={() => touch("topP")}
                onChange={(event) => set("topP", event.target.value)}
              />
            </Field>
            {model ? (
              <div className="sm:col-span-2">
                <Switch
                  checked={values.available}
                  disabled={busy}
                  label="Available for chat"
                  onChange={(next) => set("available", next)}
                />
              </div>
            ) : null}
          </>
        ) : null}

        {error ? (
          <div className="flex items-start gap-3 sm:col-span-2" ref={errorRef} role="alert" tabIndex={-1}>
            <p className="type-caption min-w-0 flex-1 text-danger">{error}</p>
            <SettingsButton onClick={() => setError(null)}>Dismiss error</SettingsButton>
          </div>
        ) : null}
        {step === "model" && verifiesConnection ? (
          <p className="type-caption text-muted sm:col-span-2" role={submitting ? "status" : undefined}>
            {submitting
              ? "Checking a short greeting before saving the model."
              : "Before saving, a short greeting checks this model connection. This uses a small number of tokens."}
          </p>
        ) : null}
      </div>
    </Modal>
  );
}

function buildCreate(values: FormValues): CustomModelCreateInput {
  return {
    base_url: values.baseUrl.trim(),
    secret: values.secret,
    upstream_model_id: values.upstreamModelId.trim(),
    display_name: values.displayName.trim(),
    context_length: Number(values.contextLength),
    max_output_tokens: Number(values.maxOutputTokens),
    supports_tools: values.supportsTools,
    supports_vision: values.supportsVision,
    sampling: {
      temperature: values.temperature.trim() ? Number(values.temperature) : null,
      top_p: values.topP.trim() ? Number(values.topP) : null,
    },
  };
}

function buildUpdate(model: CustomModelView, values: FormValues): CustomModelUpdateInput {
  const input: CustomModelUpdateInput = {};
  const create = buildCreate(values);
  if (create.base_url !== model.base_url) input.base_url = create.base_url;
  if (values.secret.trim()) input.secret = values.secret;
  if (create.upstream_model_id !== model.upstream_model_id) input.upstream_model_id = create.upstream_model_id;
  if (create.display_name !== model.display_name) input.display_name = create.display_name;
  if (create.context_length !== model.context_length) input.context_length = create.context_length;
  if (create.max_output_tokens !== model.max_output_tokens) input.max_output_tokens = create.max_output_tokens;
  if (create.supports_tools !== model.supports_tools) input.supports_tools = create.supports_tools;
  if (create.supports_vision !== model.supports_vision) input.supports_vision = create.supports_vision;
  if (create.sampling.temperature !== model.sampling.temperature || create.sampling.top_p !== model.sampling.top_p)
    input.sampling = create.sampling;
  if (values.available !== (model.is_enabled && model.is_active)) {
    input.is_enabled = values.available;
    input.is_active = values.available;
  }
  return input;
}
