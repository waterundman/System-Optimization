import type {
  Authority,
  ContextExclusion,
  ContextItem,
  ContextPacket,
  ContextPacketId,
  ContextTier,
  OperationIntent,
  ProviderLocality,
} from "@optimizer/protocol";
import { validateOperationIntent } from "@optimizer/protocol";
import { DomainError } from "./errors.ts";
import type { ContextCandidate, KernelPorts } from "./ports.ts";
import type { OperationProfile } from "./profiles.ts";
import { stableStringify } from "./stable-json.ts";

const tierOrder: readonly ContextTier[] = [
  "L0_TARGET",
  "L1_LOCAL",
  "L2_STRUCTURAL",
  "L3_KNOWLEDGE",
  "L4_STYLE_GLOBAL",
];

const authorityScore: Readonly<Record<Authority, number>> = {
  user_confirmed: 1,
  source_derived: 0.75,
  model_inferred: 0.35,
  external_untrusted: 0.1,
};

interface ScoredCandidate extends ContextCandidate {
  readonly estimatedTokens: number;
  readonly score: number;
  readonly mandatory: boolean;
}

export interface CompileContextRequest {
  readonly intent: OperationIntent;
  readonly profile: OperationProfile;
  readonly providerLocality: ProviderLocality;
  readonly modelLimit: number;
  readonly reservedOverhead: number;
}

function clamp01(value: number | undefined, fallback: number): number {
  if (value === undefined) return fallback;
  return Math.max(0, Math.min(1, value));
}

function compareCandidates(a: ScoredCandidate, b: ScoredCandidate): number {
  if (a.mandatory !== b.mandatory) return a.mandatory ? -1 : 1;
  if (a.score !== b.score) return b.score - a.score;
  const tierDelta = tierOrder.indexOf(a.tier) - tierOrder.indexOf(b.tier);
  if (tierDelta !== 0) return tierDelta;
  const refDelta = a.sourceRef.localeCompare(b.sourceRef);
  return refDelta !== 0 ? refDelta : a.sourceHash.localeCompare(b.sourceHash);
}

function exclusion(candidate: ContextCandidate, reason: ContextExclusion["reason"]): ContextExclusion {
  return { sourceRef: candidate.sourceRef, sourceHash: candidate.sourceHash, reason };
}

export class ContextCompiler {
  static readonly version = "1.0.0";
  private readonly ports: KernelPorts;

  constructor(ports: KernelPorts) {
    this.ports = ports;
  }

  async compile(request: CompileContextRequest): Promise<ContextPacket> {
    const validation = validateOperationIntent(request.intent);
    if (!validation.ok) {
      throw new DomainError({
        code: "OPERATION_INTENT_INVALID",
        category: "validation",
        message: "OperationIntent failed validation",
        details: { issues: validation.issues },
      });
    }

    if (!Number.isInteger(request.modelLimit) || request.modelLimit <= 0 || request.reservedOverhead < 0) {
      throw new DomainError({
        code: "CTX_BUDGET_INVALID",
        category: "validation",
        message: "Model and overhead budgets must be non-negative integers",
      });
    }

    const inputBudget = request.modelLimit - request.intent.output.maxTokens - request.reservedOverhead;
    if (inputBudget <= 0) {
      throw new DomainError({
        code: "CTX_BUDGET_EXCEEDED",
        category: "validation",
        message: "No input budget remains after reserving output and protocol overhead",
        details: { modelLimit: request.modelLimit, reservedOutput: request.intent.output.maxTokens, reservedOverhead: request.reservedOverhead },
      });
    }

    const batches = await Promise.all(
      this.ports.contextSources.map(async (source) => ({ sourceId: source.id, candidates: await source.collect(request.intent) })),
    );
    const candidates = batches
      .sort((a, b) => a.sourceId.localeCompare(b.sourceId))
      .flatMap((batch) => batch.candidates);

    const exclusions: ContextExclusion[] = [];
    const scored: ScoredCandidate[] = [];

    for (const candidate of candidates) {
      if (["archived", "rejected", "deleted"].includes(candidate.status)) {
        exclusions.push(exclusion(candidate, "INELIGIBLE_STATUS"));
        continue;
      }
      if (candidate.status === "draft" && !candidate.selectedByUser) {
        exclusions.push(exclusion(candidate, "DRAFT_NOT_SELECTED"));
        continue;
      }
      if (request.providerLocality === "remote" && candidate.sensitivity === "never_send") {
        exclusions.push(exclusion(candidate, "POLICY_DENIED"));
        continue;
      }

      const estimatedTokens = Math.max(0, Math.ceil(this.ports.tokenizer.estimate(candidate.content)));
      const score =
        request.profile.tierWeights[candidate.tier] +
        clamp01(candidate.signals?.relevance, 0.5) * 30 +
        clamp01(candidate.signals?.structuralProximity, 0.5) * 20 +
        authorityScore[candidate.authority] * 15 +
        clamp01(candidate.signals?.freshness, 1) * 10 +
        (candidate.selectedByUser ? 25 : 0) -
        clamp01(candidate.signals?.risk, 0) * 20;

      scored.push({ ...candidate, estimatedTokens, score, mandatory: candidate.mandatory === true });
    }

    scored.sort(compareCandidates);
    const unique: ScoredCandidate[] = [];
    const seen = new Set<string>();
    for (const candidate of scored) {
      const key = `${candidate.sourceRef}\u0000${candidate.sourceHash}\u0000${candidate.renderMode}`;
      if (seen.has(key)) {
        exclusions.push(exclusion(candidate, "DUPLICATE"));
        continue;
      }
      seen.add(key);
      unique.push(candidate);
    }

    const target = unique.find((candidate) => candidate.tier === "L0_TARGET");
    if (!target) {
      throw new DomainError({
        code: "CTX_TARGET_MISSING",
        category: "user_action",
        message: "No L0 target candidate was produced for the operation",
      });
    }

    const mandatory = unique.filter((candidate) => candidate.mandatory);
    const mandatoryTokens = mandatory.reduce((sum, candidate) => sum + candidate.estimatedTokens, 0);
    if (mandatoryTokens > inputBudget) {
      throw new DomainError({
        code: "CTX_BUDGET_EXCEEDED",
        category: "validation",
        message: "Mandatory target and constraints exceed the available input budget",
        details: { mandatoryTokens, inputBudget },
      });
    }

    const tierCaps = Object.fromEntries(
      tierOrder.map((tier) => [tier, Math.floor(inputBudget * request.profile.tierFractions[tier])]),
    ) as Record<ContextTier, number>;
    const tierUsed = Object.fromEntries(tierOrder.map((tier) => [tier, 0])) as Record<ContextTier, number>;
    const selected: ScoredCandidate[] = [];
    let used = 0;

    for (const candidate of mandatory) {
      selected.push(candidate);
      used += candidate.estimatedTokens;
      tierUsed[candidate.tier] += candidate.estimatedTokens;
    }

    const optional = unique.filter((candidate) => !candidate.mandatory);
    const deferred: ScoredCandidate[] = [];
    for (const candidate of optional) {
      if (used + candidate.estimatedTokens > inputBudget) {
        exclusions.push(exclusion(candidate, "TOTAL_BUDGET"));
        continue;
      }
      if (tierUsed[candidate.tier] + candidate.estimatedTokens <= tierCaps[candidate.tier]) {
        selected.push(candidate);
        used += candidate.estimatedTokens;
        tierUsed[candidate.tier] += candidate.estimatedTokens;
      } else {
        deferred.push(candidate);
      }
    }

    for (const candidate of deferred) {
      if (request.profile.allowTierBorrow && used + candidate.estimatedTokens <= inputBudget) {
        selected.push(candidate);
        used += candidate.estimatedTokens;
        tierUsed[candidate.tier] += candidate.estimatedTokens;
      } else {
        exclusions.push(exclusion(candidate, request.profile.allowTierBorrow ? "TOTAL_BUDGET" : "TIER_BUDGET"));
      }
    }

    selected.sort((a, b) => {
      const tierDelta = tierOrder.indexOf(a.tier) - tierOrder.indexOf(b.tier);
      return tierDelta !== 0 ? tierDelta : compareCandidates(a, b);
    });
    exclusions.sort((a, b) => a.sourceRef.localeCompare(b.sourceRef) || a.reason.localeCompare(b.reason));

    const items: ContextItem[] = selected.map((candidate) => ({
      id: candidate.id,
      sourceRef: candidate.sourceRef,
      sourceHash: candidate.sourceHash,
      tier: candidate.tier,
      status: candidate.status,
      authority: candidate.authority,
      sensitivity: candidate.sensitivity,
      renderMode: candidate.renderMode,
      reasonCodes: [...candidate.reasonCodes],
      estimatedTokens: candidate.estimatedTokens,
      score: Number(candidate.score.toFixed(6)),
      mandatory: candidate.mandatory,
      content: candidate.content,
    }));

    const packetId = this.ports.ids.next("ctx") as ContextPacketId;
    const compiledAt = this.ports.clock.now();
    const packetWithoutHash = {
      schemaVersion: 1 as const,
      id: packetId,
      operationIntentId: request.intent.id,
      projectId: request.intent.projectId,
      baseCommitId: request.intent.baseCommitId,
      providerLocality: request.providerLocality,
      items,
      exclusions,
      budget: {
        modelLimit: request.modelLimit,
        reservedOutput: request.intent.output.maxTokens,
        reservedOverhead: request.reservedOverhead,
        inputBudget,
        estimatedInput: used,
        tokenizer: this.ports.tokenizer.id,
      },
      compilerVersion: ContextCompiler.version,
      operationProfileVersion: request.profile.version,
      compiledAt,
    };
    const packetHash = await this.ports.hasher.sha256(stableStringify(packetWithoutHash));
    const packet: ContextPacket = { ...packetWithoutHash, packetHash };

    this.ports.telemetry?.emit({
      name: "context.compiled",
      attributes: {
        operationType: request.intent.type,
        selectedItems: items.length,
        excludedItems: exclusions.length,
        estimatedTokens: used,
        providerLocality: request.providerLocality,
      },
    });
    return packet;
  }
}
