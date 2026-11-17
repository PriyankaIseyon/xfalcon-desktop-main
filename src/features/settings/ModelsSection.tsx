import { useCallback, useEffect, useState } from "react";
import { Badge } from "../../ui/Badge";
import { Select } from "../../ui/Select";
import { TruncatedText } from "../../ui/TruncatedText";
import { toast } from "../../ui/toastStore";
import { getAuthAccountGeneration } from "../auth/authStore";
import { saveDefaultModel } from "../models/modelNative";
import { invalidateModelCatalog, useModelCatalog } from "../models/useModelCatalog";
import { CustomModelsGroup } from "./CustomModelsGroup";
import { SettingsGroup, SettingsRow } from "./SettingsLayout";
import { MODEL_SOURCE_LABEL, MODEL_SOURCE_ORDER } from "./settingsCopy";

export function ModelsSection() {
  const { catalog, loading, error } = useModelCatalog(null);
  const accountGeneration = getAuthAccountGeneration();
  const [savingGeneration, setSavingGeneration] = useState<number | null>(null);
  const [selection, setSelection] = useState<{ accountGeneration: number; modelId: string } | null>(null);
  const saving = savingGeneration === accountGeneration;
  const value =
    (selection?.accountGeneration === accountGeneration ? selection.modelId : null) ?? catalog?.selected_model_id;

  useEffect(() => {
    setSelection(null);
    setSavingGeneration(null);
  }, [catalog]);

  const choose = useCallback(async (modelId: string) => {
    const accountGeneration = getAuthAccountGeneration();
    setSelection({ accountGeneration, modelId });
    setSavingGeneration(accountGeneration);
    try {
      const saved = await saveDefaultModel(modelId);
      invalidateModelCatalog();
      if (accountGeneration !== getAuthAccountGeneration()) return;
      setSelection({ accountGeneration, modelId: saved.selected_model_id });
    } catch {
      if (accountGeneration !== getAuthAccountGeneration()) return;
      setSelection(null);
      toast.error("Could not save the default model.");
    } finally {
      if (accountGeneration === getAuthAccountGeneration()) setSavingGeneration(null);
    }
  }, []);

  // Sorted under their source heading. The sort is stable, so catalog order holds within a heading.
  const models = [...(catalog?.models ?? [])].sort(
    (a, b) =>
      MODEL_SOURCE_ORDER.indexOf(MODEL_SOURCE_LABEL[a.provider].group) -
      MODEL_SOURCE_ORDER.indexOf(MODEL_SOURCE_LABEL[b.provider].group),
  );

  return (
    <>
      <SettingsGroup id="settings-default-model" aria-label="Default model">
        <SettingsRow
          label="Default model"
          hint="New chats use this model and its key. Existing chats keep their model."
          labelId="settings-default-model-label"
        >
          <Select
            aria-labelledby="settings-default-model-label"
            className="w-full sm:w-64"
            disabled={loading || saving || !catalog || catalog.models.length === 0}
            onChange={(modelId) => void choose(modelId)}
            options={models.map((model) => ({
              label: (
                <span className="flex min-w-0 items-center gap-2">
                  <TruncatedText as="span" className="min-w-0 flex-1 text-foreground" text={model.display_name}>
                    {model.display_name}
                  </TruncatedText>
                  <Badge aria-hidden className="shrink-0" tone="neutral">
                    {MODEL_SOURCE_LABEL[model.provider].badge}
                  </Badge>
                  <span className="sr-only">, {MODEL_SOURCE_LABEL[model.provider].spoken}</span>
                </span>
              ),
              group: MODEL_SOURCE_LABEL[model.provider].group,
              value: model.id,
            }))}
            placeholder={error ?? (loading ? "Loading models…" : "No available models.")}
            value={value}
          />
        </SettingsRow>
      </SettingsGroup>
      <CustomModelsGroup onCatalogChanged={invalidateModelCatalog} />
    </>
  );
}
