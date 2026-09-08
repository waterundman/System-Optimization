import type { ContextTier, OperationType } from "@optimizer/protocol";

export interface OperationProfile {
  readonly version: string;
  readonly tierFractions: Readonly<Record<ContextTier, number>>;
  readonly tierWeights: Readonly<Record<ContextTier, number>>;
  readonly allowTierBorrow: boolean;
}

const weights: Readonly<Record<ContextTier, number>> = {
  L0_TARGET: 100,
  L1_LOCAL: 80,
  L2_STRUCTURAL: 65,
  L3_KNOWLEDGE: 50,
  L4_STYLE_GLOBAL: 35,
};

const fractions = (
  target: number,
  local: number,
  structural: number,
  knowledge: number,
  style: number,
): Readonly<Record<ContextTier, number>> => ({
  L0_TARGET: target,
  L1_LOCAL: local,
  L2_STRUCTURAL: structural,
  L3_KNOWLEDGE: knowledge,
  L4_STYLE_GLOBAL: style,
});

export const coreOperationProfiles: Readonly<Record<OperationType, OperationProfile>> = {
  polish: { version: "1.0.0", tierFractions: fractions(0.45, 0.35, 0.08, 0.04, 0.08), tierWeights: weights, allowTierBorrow: true },
  continue_scene: { version: "1.0.0", tierFractions: fractions(0.25, 0.25, 0.20, 0.22, 0.08), tierWeights: weights, allowTierBorrow: true },
  compress: { version: "1.0.0", tierFractions: fractions(0.60, 0.20, 0.10, 0.05, 0.05), tierWeights: weights, allowTierBorrow: true },
  expand: { version: "1.0.0", tierFractions: fractions(0.40, 0.20, 0.15, 0.15, 0.10), tierWeights: weights, allowTierBorrow: true },
  critique: { version: "1.0.0", tierFractions: fractions(0.15, 0.10, 0.35, 0.25, 0.15), tierWeights: weights, allowTierBorrow: true },
};

