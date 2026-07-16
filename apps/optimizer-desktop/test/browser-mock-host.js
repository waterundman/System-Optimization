(() => {
  const recentWelcome = new URLSearchParams(window.location.search).has("recentWelcome");
  if (new URLSearchParams(window.location.search).has("autoDialogs")) {
    window.prompt = (message) => message.includes("新的章节") ? "第二章：雨夜" : "第二章";
    window.confirm = () => true;
  }
  const block = {
    id: "block-visual-1",
    documentId: "document-visual-1",
    kind: "paragraph",
    orderKey: "a0",
    content: { type: "paragraph", content: [{ type: "text", text: "雨停了。" }] },
    plainText: "雨停了。",
    contentHash: `sha256:${"0".repeat(64)}`,
    revision: 0,
    locked: false,
  };
  const workspace = {
    schemaVersion: 1,
    projectId: "project-visual-1",
    mainBranchId: "branch-visual-1",
    headCommitId: "commit-visual-1",
    revision: 0,
    documents: [{
      id: "document-visual-1",
      parentId: null,
      kind: "chapter",
      title: "雾港来信",
      orderKey: "a0",
      revision: 0,
    }],
    blocks: [block],
  };
  const styleSamples = [{
    schemaVersion: 1,
    id: "style-visual-1",
    title: "克制的短句",
    content: "风从站台尽头吹来。灯没有熄。",
    contentHash: `sha256:${"2".repeat(64)}`,
    status: "canonical",
    sensitivity: "local_sensitive",
    revision: 0,
    createdAt: "2026-07-15T00:00:00Z",
    updatedAt: "2026-07-15T00:00:00Z",
  }];
  const knowledgeItems = [{
    schemaVersion: 1,
    id: "knowledge-visual-1",
    kind: "fact",
    title: "主角视觉",
    content: "主角左眼失明",
    contentHash: `sha256:${"5".repeat(64)}`,
    status: "canonical",
    authority: "user_confirmed",
    sensitivity: "local_sensitive",
    severity: null,
    revision: 0,
    createdAt: "2026-07-16T00:00:00Z",
    updatedAt: "2026-07-16T00:00:00Z",
  }];
  const archivedDocuments = [];
  const archivedBlocks = new Map();
  const session = {
    schemaVersion: 1,
    isOpen: !recentWelcome,
    project: {
      schemaVersion: 1,
      projectId: workspace.projectId,
      mainBranchId: workspace.mainBranchId,
      title: "雾港来信",
      language: "zh-CN",
      directory: "W:\\写作\\雾港来信.optimizer",
      databaseSchemaVersion: 6,
      headCommitId: workspace.headCommitId,
      revision: 0,
      createdAt: "2026-07-15T00:00:00Z",
      updatedAt: "2026-07-15T00:00:00Z",
    },
  };
  class Channel {
    onmessage = null;
  }
  let authorizedModelRequest = null;
  let summariesReady = false;
  async function contentHash(value) {
    const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(value));
    return `sha256:${[...new Uint8Array(digest)].map((byte) => byte.toString(16).padStart(2, "0")).join("")}`;
  }
  function documentMutationResponse(reason) {
    summariesReady = false;
    const previousHeadCommitId = workspace.headCommitId;
    workspace.revision += 1;
    workspace.headCommitId = `commit-visual-${reason}-${workspace.revision}`;
    session.project.revision = workspace.revision;
    session.project.headCommitId = workspace.headCommitId;
    return {
      schemaVersion: 1,
      commitId: workspace.headCommitId,
      previousHeadCommitId,
      headCommitId: workspace.headCommitId,
      projectRevision: workspace.revision,
      workspace: structuredClone(workspace),
    };
  }
  async function invoke(command, args = {}) {
    if (command === "get_project_session") return structuredClone(session);
    if (command === "list_recent_projects") {
      return [{
        schemaVersion: 1,
        projectId: workspace.projectId,
        title: session.project.title,
        language: session.project.language,
        directory: session.project.directory,
        lastOpenedAt: "2026-07-16T00:00:00Z",
        available: true,
      }];
    }
    if (command === "open_recent_project") {
      if (args.input.projectId !== workspace.projectId) throw { code: "RECENT_PROJECT_NOT_FOUND", message: "Missing recent project" };
      session.isOpen = true;
      return structuredClone(session);
    }
    if (command === "remove_recent_project") {
      return { schemaVersion: 1, projectId: args.input.projectId, changed: true };
    }
    if (command === "close_project") {
      session.isOpen = false;
      return { schemaVersion: 1, isOpen: false, project: null };
    }
    if (command === "get_project_workspace") return structuredClone(workspace);
    if (command === "list_archived_documents") return structuredClone(archivedDocuments);
    if (command === "list_summary_invalidations") {
      if (summariesReady) return [];
      return [
        { scopeType: "project", scopeId: workspace.projectId },
        ...workspace.documents.map((item) => ({ scopeType: "document", scopeId: item.id })),
        ...workspace.blocks.map((item) => ({ scopeType: "block", scopeId: item.id })),
      ].map((item) => ({
        schemaVersion: 1,
        ...item,
        sourceCommitId: workspace.headCommitId,
        reason: "visual_mock",
        invalidationCount: 1,
        createdAt: "2026-07-16T00:00:00Z",
      }));
    }
    if (command === "refresh_summaries") {
      const processed = summariesReady ? 0 : 1 + workspace.documents.length + workspace.blocks.length;
      summariesReady = true;
      return {
        schemaVersion: 1,
        processed,
        remaining: 0,
        providerId: "optimizer-local",
        model: "extractive-summary-v1",
      };
    }
    if (command === "get_summary_context") {
      if (!summariesReady || args.input.baseCommitId !== workspace.headCommitId) return [];
      const target = workspace.blocks.find((item) => item.id === args.input.targetBlockId);
      if (!target || target.revision !== args.input.targetBlockRevision || target.contentHash !== args.input.targetBlockHash) {
        throw { code: "SUMMARY_CONTEXT_BINDING_INVALID", message: "Target changed" };
      }
      const document = workspace.documents.find((item) => item.id === target.documentId);
      const payloads = [
        {
          id: `summary-document-${document.id}`,
          sourceRef: `summary:document:${document.id}@${workspace.headCommitId}`,
          sourceCommitId: workspace.headCommitId,
          tier: "L2_STRUCTURAL",
          reasonCodes: ["CURRENT_DOCUMENT_SUMMARY"],
          content: `章节“${document.title}”摘要：${target.plainText}`,
          revision: 0,
        },
        {
          id: `summary-project-${workspace.projectId}`,
          sourceRef: `summary:project:${workspace.projectId}@${workspace.headCommitId}`,
          sourceCommitId: workspace.headCommitId,
          tier: "L3_KNOWLEDGE",
          reasonCodes: ["PROJECT_SUMMARY"],
          content: `项目“${session.project.title}”摘要：${workspace.documents.map((item) => item.title).join("、")}`,
          revision: 0,
        },
      ];
      return Promise.all(payloads.map(async (item) => ({
        ...item,
        sourceHash: await contentHash(item.content),
        status: "canonical",
        authority: "source_derived",
        sensitivity: "local_sensitive",
        renderMode: "summary",
        generatedAt: "2026-07-16T00:00:00Z",
      })));
    }
    if (command === "get_knowledge_context") {
      if (args.input.baseCommitId !== workspace.headCommitId) return [];
      const target = workspace.blocks.find((item) => item.id === args.input.targetBlockId);
      if (!target || target.revision !== args.input.targetBlockRevision || target.contentHash !== args.input.targetBlockHash) {
        throw { code: "KNOWLEDGE_CONTEXT_BINDING_INVALID", message: "Target changed" };
      }
      return Promise.all(knowledgeItems
        .filter((item) => item.status === "canonical")
        .map(async (item) => {
          const label = item.kind === "fact" ? "事实" : (item.severity === "hard" ? "硬约束" : "软约束");
          const content = `${label}【${item.title}】：${item.content}`;
          return {
            id: `knowledge-${item.id}-r${item.revision}`,
            sourceRef: `knowledge:${item.kind}:${item.id}@r${item.revision}`,
            sourceHash: await contentHash(content),
            sourceCommitId: workspace.headCommitId,
            tier: "L3_KNOWLEDGE",
            status: "canonical",
            authority: item.authority,
            sensitivity: item.sensitivity,
            renderMode: "constraint",
            reasonCodes: [item.kind === "fact" ? "CANONICAL_FACT" : `PROJECT_${item.severity.toUpperCase()}_CONSTRAINT`],
            content,
            revision: item.revision,
            generatedAt: item.updatedAt,
          };
        }));
    }
    if (command === "create_document") {
      const index = workspace.documents.length + 1;
      const document = {
        id: `document-visual-${index}`,
        parentId: args.input.parentDocumentId ?? null,
        kind: "chapter",
        title: args.input.title.trim(),
        orderKey: `z-document-visual-${index}`,
        revision: 0,
      };
      const createdBlock = {
        id: `block-visual-${index}`,
        documentId: document.id,
        kind: "paragraph",
        orderKey: "a0",
        content: { type: "paragraph", content: [] },
        plainText: args.input.initialText,
        contentHash: `sha256:${"4".repeat(64)}`,
        revision: 0,
        locked: false,
      };
      workspace.documents.push(document);
      workspace.blocks.push(createdBlock);
      summariesReady = false;
      workspace.revision += 1;
      workspace.headCommitId = `commit-visual-${index}`;
      return {
        schemaVersion: 1,
        commitId: workspace.headCommitId,
        previousHeadCommitId: "commit-visual-1",
        projectRevision: workspace.revision,
        document: structuredClone(document),
        block: structuredClone(createdBlock),
      };
    }
    if (command === "rename_document") {
      const document = workspace.documents.find((item) => item.id === args.input.documentId);
      if (!document || document.revision !== args.input.expectedRevision) throw { code: "CONFLICT", message: "Document changed" };
      document.title = args.input.title.trim();
      document.revision += 1;
      return documentMutationResponse("rename");
    }
    if (command === "reorder_document") {
      const document = workspace.documents.find((item) => item.id === args.input.documentId);
      const siblings = workspace.documents
        .filter((item) => item.parentId === document?.parentId)
        .sort((left, right) => left.orderKey.localeCompare(right.orderKey));
      const index = siblings.findIndex((item) => item.id === args.input.documentId);
      const target = args.input.direction === "up" ? index - 1 : index + 1;
      if (index < 0 || target < 0 || target >= siblings.length) throw { code: "NO_CHANGES", message: "No changes" };
      siblings.splice(target, 0, siblings.splice(index, 1)[0]);
      siblings.forEach((item, position) => {
        item.orderKey = `d-${String(position).padStart(8, "0")}-visual`;
        item.revision += 1;
      });
      return documentMutationResponse("reorder");
    }
    if (command === "change_document_depth") {
      const document = workspace.documents.find((item) => item.id === args.input.documentId);
      if (!document || document.revision !== args.input.expectedRevision) throw { code: "CONFLICT", message: "Document changed" };
      if (args.input.direction === "indent") {
        const siblings = workspace.documents
          .filter((item) => item.parentId === document.parentId)
          .sort((left, right) => left.orderKey.localeCompare(right.orderKey));
        const index = siblings.findIndex((item) => item.id === document.id);
        if (index <= 0) throw { code: "NO_CHANGES", message: "No changes" };
        document.parentId = siblings[index - 1].id;
      } else {
        const parent = workspace.documents.find((item) => item.id === document.parentId);
        if (!parent) throw { code: "NO_CHANGES", message: "No changes" };
        document.parentId = parent.parentId;
      }
      const destination = workspace.documents
        .filter((item) => item.parentId === document.parentId)
        .sort((left, right) => left.orderKey.localeCompare(right.orderKey));
      destination.forEach((item, position) => {
        item.orderKey = `d-${String(position).padStart(8, "0")}-depth`;
        item.revision += 1;
      });
      return documentMutationResponse("reparent");
    }
    if (command === "set_document_archived") {
      if (args.input.archived) {
        const index = workspace.documents.findIndex((item) => item.id === args.input.documentId);
        const document = workspace.documents[index];
        if (!document || document.revision !== args.input.expectedRevision) throw { code: "CONFLICT", message: "Document changed" };
        workspace.documents.splice(index, 1);
        document.revision += 1;
        archivedDocuments.push({ ...document, schemaVersion: 1, archivedAt: "2026-07-16T00:00:00Z" });
        const blocks = workspace.blocks.filter((item) => item.documentId === document.id);
        archivedBlocks.set(document.id, blocks);
        workspace.blocks = workspace.blocks.filter((item) => item.documentId !== document.id);
        return documentMutationResponse("archive");
      }
      const index = archivedDocuments.findIndex((item) => item.id === args.input.documentId);
      const archived = archivedDocuments[index];
      if (!archived || archived.revision !== args.input.expectedRevision) throw { code: "CONFLICT", message: "Document changed" };
      archivedDocuments.splice(index, 1);
      const { archivedAt: _archivedAt, schemaVersion: _schemaVersion, ...document } = archived;
      document.revision += 1;
      workspace.documents.push(document);
      workspace.documents.sort((left, right) => left.orderKey.localeCompare(right.orderKey));
      workspace.blocks.push(...(archivedBlocks.get(document.id) ?? []));
      archivedBlocks.delete(document.id);
      return documentMutationResponse("restore");
    }
    if (command === "export_markdown") {
      return {
        schemaVersion: 1,
        path: "W:\\写作\\雾港来信.optimizer\\exports\\optimizer-export-visual.md",
        bytes: 128,
        documents: workspace.documents.length,
      };
    }
    if (command === "list_style_samples") return structuredClone(styleSamples);
    if (command === "create_style_sample") {
      const created = {
        schemaVersion: 1,
        id: `style-visual-${styleSamples.length + 1}`,
        title: args.input.title,
        content: args.input.content,
        contentHash: `sha256:${"3".repeat(64)}`,
        status: "canonical",
        sensitivity: args.input.sensitivity,
        revision: 0,
        createdAt: "2026-07-15T00:01:00Z",
        updatedAt: "2026-07-15T00:01:00Z",
      };
      styleSamples.unshift(created);
      return structuredClone(created);
    }
    if (command === "set_style_sample_status") {
      const sample = styleSamples.find((item) => item.id === args.input.id);
      Object.assign(sample, { status: args.input.status, revision: sample.revision + 1 });
      return structuredClone(sample);
    }
    if (command === "list_knowledge_items") return structuredClone(knowledgeItems);
    if (command === "create_knowledge_item") {
      const created = {
        schemaVersion: 1,
        id: `knowledge-visual-${knowledgeItems.length + 1}`,
        kind: args.input.kind,
        title: args.input.title,
        content: args.input.content,
        contentHash: await contentHash(args.input.content),
        status: "canonical",
        authority: "user_confirmed",
        sensitivity: args.input.sensitivity,
        severity: args.input.severity,
        revision: 0,
        createdAt: "2026-07-16T00:01:00Z",
        updatedAt: "2026-07-16T00:01:00Z",
      };
      knowledgeItems.unshift(created);
      return structuredClone(created);
    }
    if (command === "set_knowledge_item_status") {
      const item = knowledgeItems.find((candidate) => candidate.id === args.input.id);
      if (!item || item.revision !== args.input.expectedRevision) throw { code: "CONFLICT", message: "Knowledge changed" };
      Object.assign(item, {
        status: args.input.status,
        revision: item.revision + 1,
        updatedAt: "2026-07-16T00:02:00Z",
      });
      return structuredClone(item);
    }
    if (command === "has_provider_secret") {
      return { schemaVersion: 1, reference: args.reference, exists: true };
    }
    if (command === "store_provider_secret") {
      return { schemaVersion: 1, reference: args.reference, changed: true, exists: true };
    }
    if (command === "delete_provider_secret") {
      return { schemaVersion: 1, reference: args.reference, changed: true, exists: false };
    }
    if (command === "list_ollama_models") {
      return {
        schemaVersion: 1,
        endpoint: "127.0.0.1:11434/v1",
        models: [
          { id: "qwen3:8b", created: 2, ownedBy: "library" },
          { id: "llama3.2", created: 1, ownedBy: "library" },
        ],
      };
    }
    if (command === "authorize_model_request") {
      authorizedModelRequest = structuredClone(args.input.request);
      return {
        schemaVersion: 1,
        authorizationId: "model-auth-visual-1",
        requestId: authorizedModelRequest.requestId,
        providerId: authorizedModelRequest.configuration.providerId,
        expiresAt: "2026-07-15T00:02:00Z",
      };
    }
    if (command === "execute_authorized_model_stream") {
      if (args.authorizationId !== "model-auth-visual-1" || !authorizedModelRequest) {
        throw new Error("Model execution did not consume the visual authorization");
      }
      const output = JSON.stringify({
        schemaVersion: 1,
        kind: "replacement",
        replacementText: "她推开车站的门。",
        summary: "延续雨后的车站场景",
      });
      for (const event of [
        { type: "start", requestId: authorizedModelRequest.requestId, id: "response-visual-1", providerId: "deepseek", model: "deepseek-v4-flash" },
        { type: "text_delta", text: output },
        { type: "finish", reason: "stop" },
      ]) args.onEvent.onmessage?.(event);
      return {
        schemaVersion: 1,
        requestId: authorizedModelRequest.requestId,
        responseId: "response-visual-1",
        providerId: "deepseek",
        model: "deepseek-v4-flash",
        content: output,
        reasoningContent: "",
        finishReason: "stop",
      };
    }
    if (command === "persist_operation_bundle") {
      return { schemaVersion: 1, runId: "run-visual-1", state: "review", artifactId: "proposal-visual-1" };
    }
    if (command === "append_review_event") {
      return { schemaVersion: 1, proposalId: "proposal-visual-1", revision: 1, status: "ready", runId: "run-visual-1", operationState: "review" };
    }
    if (command === "apply_reviewed_proposal") {
      const previousHeadCommitId = workspace.headCommitId;
      const next = {
        ...block,
        content: { type: "paragraph", content: [{ type: "text", text: "雨停了。她推开车站的门。" }] },
        plainText: "雨停了。她推开车站的门。",
        contentHash: `sha256:${"1".repeat(64)}`,
        revision: 1,
      };
      Object.assign(block, next);
      workspace.headCommitId = "commit-visual-2";
      workspace.revision = 1;
      session.project.headCommitId = workspace.headCommitId;
      session.project.revision = workspace.revision;
      summariesReady = false;
      return {
        schemaVersion: 1,
        proposalId: args.input.proposalId,
        runId: "run-visual-1",
        reviewRevision: 2,
        acceptedHunks: 1,
        rejectedHunks: 0,
        save: {
          schemaVersion: 1,
          editId: "edit-visual-1",
          commitId: "commit-visual-2",
          previousHeadCommitId,
          headCommitId: "commit-visual-2",
          projectRevision: 1,
          block: next,
        },
      };
    }
    if (command === "cancel_model_request") {
      return { schemaVersion: 1, requestId: args.requestId, cancelled: true };
    }
    if (command === "get_version_history") {
      return { schemaVersion: 1, projectId: workspace.projectId, headCommitId: workspace.headCommitId, commits: [], checkpoints: [] };
    }
    throw new Error(`Mock host did not implement ${command}`);
  }
  window.__TAURI__ = { core: { Channel, invoke } };
})();
