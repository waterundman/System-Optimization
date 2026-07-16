(() => {
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
  const archivedDocuments = [];
  const archivedBlocks = new Map();
  const session = {
    schemaVersion: 1,
    isOpen: true,
    project: {
      schemaVersion: 1,
      projectId: workspace.projectId,
      mainBranchId: workspace.mainBranchId,
      title: "雾港来信",
      language: "zh-CN",
      directory: "W:\\写作\\雾港来信.optimizer",
      databaseSchemaVersion: 5,
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
  function documentMutationResponse(reason) {
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
    if (command === "get_project_workspace") return structuredClone(workspace);
    if (command === "list_archived_documents") return structuredClone(archivedDocuments);
    if (command === "list_summary_invalidations") {
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
    if (command === "create_document") {
      const index = workspace.documents.length + 1;
      const document = {
        id: `document-visual-${index}`,
        parentId: null,
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
      workspace.documents.sort((left, right) => left.orderKey.localeCompare(right.orderKey));
      const index = workspace.documents.findIndex((item) => item.id === args.input.documentId);
      const target = args.input.direction === "up" ? index - 1 : index + 1;
      if (index < 0 || target < 0 || target >= workspace.documents.length) throw { code: "NO_CHANGES", message: "No changes" };
      workspace.documents.splice(target, 0, workspace.documents.splice(index, 1)[0]);
      workspace.documents.forEach((document, position) => {
        document.orderKey = `d-${String(position).padStart(8, "0")}-visual`;
        document.revision += 1;
      });
      return documentMutationResponse("reorder");
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
      const next = {
        ...block,
        content: { type: "paragraph", content: [{ type: "text", text: "雨停了。她推开车站的门。" }] },
        plainText: "雨停了。她推开车站的门。",
        contentHash: `sha256:${"1".repeat(64)}`,
        revision: 1,
      };
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
          previousHeadCommitId: workspace.headCommitId,
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
