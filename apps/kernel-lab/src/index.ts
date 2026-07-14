import { ContextCompiler, coreOperationProfiles } from "../../../packages/kernel/src/index.ts";
import { ApproxTokenEstimator, FixedClock, SequentialIds, Sha256Hasher, createIntent } from "../../../packages/test-kit/src/index.ts";

const compiler = new ContextCompiler({
  tokenizer: new ApproxTokenEstimator(),
  hasher: new Sha256Hasher(),
  clock: new FixedClock(),
  ids: new SequentialIds(),
  contextSources: [
    {
      id: "document-window",
      async collect() {
        return [
          {
            id: "target",
            sourceRef: "block:scene-last-paragraph",
            sourceHash: "sha256:target",
            tier: "L0_TARGET" as const,
            status: "canonical" as const,
            authority: "user_confirmed" as const,
            sensitivity: "local_sensitive" as const,
            renderMode: "verbatim" as const,
            reasonCodes: ["OPERATION_TARGET"],
            mandatory: true,
            content: "广播在空荡的站台上响起，他下意识转向声音传来的方向。",
          },
          {
            id: "local",
            sourceRef: "block:previous-paragraph",
            sourceHash: "sha256:local",
            tier: "L1_LOCAL" as const,
            status: "canonical" as const,
            authority: "source_derived" as const,
            sensitivity: "local_sensitive" as const,
            renderMode: "verbatim" as const,
            reasonCodes: ["ADJACENT_BLOCK"],
            content: "脚步声已经消失，但候车室的玻璃门仍在轻轻晃动。",
          },
        ];
      },
    },
    {
      id: "canonical-facts",
      async collect() {
        return [
          {
            id: "fact-left-eye",
            sourceRef: "fact:character-a-left-eye",
            sourceHash: "sha256:fact-left-eye",
            tier: "L3_KNOWLEDGE" as const,
            status: "canonical" as const,
            authority: "user_confirmed" as const,
            sensitivity: "local" as const,
            renderMode: "constraint" as const,
            reasonCodes: ["SCENE_PARTICIPANT", "MANDATORY_FACT"],
            content: "角色 A 左眼失明，不能写成看见左侧细节。",
            signals: { relevance: 1, structuralProximity: 1, freshness: 1 },
          },
          {
            id: "rejected-plot",
            sourceRef: "fact:rejected-plot",
            sourceHash: "sha256:rejected",
            tier: "L3_KNOWLEDGE" as const,
            status: "rejected" as const,
            authority: "user_confirmed" as const,
            sensitivity: "local" as const,
            renderMode: "summary" as const,
            reasonCodes: ["KEYWORD_MATCH"],
            content: "已废弃剧情：角色 B 是凶手。",
          },
        ];
      },
    },
  ],
});

const packet = await compiler.compile({
  intent: createIntent(),
  profile: coreOperationProfiles.continue_scene,
  providerLocality: "remote",
  modelLimit: 4096,
  reservedOverhead: 600,
});

console.log(JSON.stringify(packet, null, 2));

