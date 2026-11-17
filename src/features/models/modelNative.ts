import type {
  ChatModelCatalogView,
  ConversationId,
  CustomModelCreateInput,
  CustomModelDiscoveryInput,
  CustomModelDiscoveryResult,
  CustomModelUpdateInput,
  CustomModelView,
} from "../../lib/generated/contracts";

export async function loadModelCatalog(conversationId: ConversationId | null): Promise<ChatModelCatalogView> {
  const native = await import("../../lib/native");
  return native.chatModelCatalog(conversationId);
}

export async function saveDefaultModel(modelId: string): Promise<ChatModelCatalogView> {
  const native = await import("../../lib/native");
  return native.chatDefaultModelSet(modelId);
}

export async function listCustomModels(): Promise<CustomModelView[]> {
  const native = await import("../../lib/native");
  return native.customModelList();
}

export async function createCustomModel(input: CustomModelCreateInput): Promise<CustomModelView> {
  const native = await import("../../lib/native");
  return native.customModelCreate(input);
}

export async function discoverCustomModels(input: CustomModelDiscoveryInput): Promise<CustomModelDiscoveryResult> {
  const native = await import("../../lib/native");
  return native.customModelDiscover(input);
}

export async function updateCustomModel(id: string, input: CustomModelUpdateInput): Promise<CustomModelView> {
  const native = await import("../../lib/native");
  return native.customModelUpdate(id, input);
}

export async function deleteCustomModel(id: string): Promise<void> {
  const native = await import("../../lib/native");
  return native.customModelDelete(id);
}
