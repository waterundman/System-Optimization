import { ProviderError } from "./errors.ts";
import type {
  ModelCompletion,
  ModelGatewayOptions,
  ModelProvider,
  ModelRequest,
  ModelStreamEvent,
  ProviderId,
  ProviderProfile,
} from "./types.ts";

export class ModelProviderRouter {
  private readonly providers = new Map<ProviderId, ModelProvider>();

  constructor(providers: readonly ModelProvider[] = []) {
    for (const provider of providers) this.register(provider);
  }

  register(provider: ModelProvider): void {
    if (this.providers.has(provider.profile.id)) {
      throw configurationError(
        provider.profile.id,
        `Provider is already registered: ${provider.profile.id}`,
      );
    }
    this.providers.set(provider.profile.id, provider);
  }

  listProfiles(): readonly ProviderProfile[] {
    return [...this.providers.values()]
      .map((provider) => provider.profile)
      .sort((left, right) => left.id.localeCompare(right.id));
  }

  provider(providerId: ProviderId): ModelProvider {
    const provider = this.providers.get(providerId);
    if (!provider) {
      throw configurationError(providerId, `Provider is not configured: ${providerId}`);
    }
    return provider;
  }

  complete(
    providerId: ProviderId,
    request: ModelRequest,
    options?: ModelGatewayOptions,
  ): Promise<ModelCompletion> {
    return this.provider(providerId).complete(request, options);
  }

  async *stream(
    providerId: ProviderId,
    request: ModelRequest,
    options?: ModelGatewayOptions,
  ): AsyncGenerator<ModelStreamEvent> {
    yield* this.provider(providerId).stream(request, options);
  }
}

function configurationError(providerId: ProviderId, message: string): ProviderError {
  return new ProviderError({
    kind: "configuration",
    providerId,
    message,
  });
}
