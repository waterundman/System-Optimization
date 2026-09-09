    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    use sha2::{Digest, Sha256};

    use super::*;

    struct TempParent(PathBuf);

    static NEXT_TEMP_DIRECTORY: AtomicU64 = AtomicU64::new(0);

    impl TempParent {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "optimizer-project-package-{}-{nonce}-{}",
                std::process::id(),
                NEXT_TEMP_DIRECTORY.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TempParent {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn spec() -> NewProjectSpec {
        NewProjectSpec {
            folder_name: "MyNovel.optimizer".into(),
            title: "My Novel".into(),
            language: "zh-CN".into(),
        }
    }

    #[test]
    fn creates_an_atomic_project_package_and_reopens_it() {
        let parent = TempParent::new();
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let info = project.info().unwrap();
        assert_eq!(info.title, "My Novel");
        assert_eq!(info.language, "zh-CN");
        assert!(info.main_branch_id.starts_with("branch-"));
        assert!(Path::new(&info.directory).join(DATABASE_FILE).is_file());
        assert!(Path::new(&info.directory).join(MANIFEST_FILE).is_file());
        for directory in ["assets", "backups", "exports"] {
            assert!(Path::new(&info.directory).join(directory).is_dir());
        }
        let project_id = info.project_id;
        drop(project);

        let reopened = OpenedProject::open(parent.0.join("MyNovel.optimizer")).unwrap();
        assert_eq!(reopened.info().unwrap().project_id, project_id);
    }

    #[test]
    fn loads_the_workspace_and_saves_a_block_as_a_versioned_commit() {
        let parent = TempParent::new();
        let mut project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let initial = project.workspace().unwrap();
        assert_eq!(initial.documents.len(), 1);
        assert_eq!(initial.blocks.len(), 1);
        let block = initial.blocks[0].clone();
        let saved = project
            .save_block(&SaveBlockSpec {
                block_id: block.id.clone(),
                expected_revision: block.revision,
                expected_hash: block.content_hash.clone(),
                content: serde_json::json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "第一段正文" }]
                }),
                plain_text: "第一段正文".into(),
            })
            .unwrap();
        assert_eq!(saved.project_revision, 1);
        assert_eq!(saved.block.revision, 1);
        assert_eq!(saved.block.plain_text, "第一段正文");
        assert_eq!(saved.head_commit_id, saved.commit_id);
        assert_ne!(saved.previous_head_commit_id, saved.commit_id);
        assert!(saved.block.content_hash.starts_with("sha256:"));

        let reloaded = project.workspace().unwrap();
        assert_eq!(reloaded.revision, 1);
        assert_eq!(reloaded.documents[0].revision, 1);
        assert_eq!(reloaded.head_commit_id, saved.commit_id);
        assert_eq!(reloaded.blocks[0], saved.block);
        assert!(matches!(
            project.save_block(&SaveBlockSpec {
                block_id: saved.block.id.clone(),
                expected_revision: saved.block.revision,
                expected_hash: saved.block.content_hash.clone(),
                content: saved.block.content.clone(),
                plain_text: saved.block.plain_text.clone(),
            }),
            Err(WorkspaceCommandError::NoChanges)
        ));
        assert!(matches!(
            project.save_block(&SaveBlockSpec {
                block_id: block.id,
                expected_revision: block.revision,
                expected_hash: block.content_hash,
                content: serde_json::json!({ "type": "paragraph", "text": "stale" }),
                plain_text: "stale".into(),
            }),
            Err(WorkspaceCommandError::Store(StoreError::Conflict { .. }))
        ));
    }

    #[test]
    fn refreshes_summary_queue_and_serves_only_current_context() {
        let parent = TempParent::new();
        let mut project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let initial = project.workspace().unwrap();
        let initial_block = initial.blocks[0].clone();
        assert_eq!(project.summary_invalidations().unwrap().len(), 3);

        let first_batch = project
            .refresh_summaries(&RefreshSummariesSpec { max_items: 2 })
            .unwrap();
        assert_eq!(first_batch.processed, 2);
        assert_eq!(first_batch.remaining, 1);
        assert_eq!(first_batch.provider_id, "optimizer-local");
        assert_eq!(first_batch.model, "extractive-summary-v1");
        let document_only = project
            .summary_context(&SummaryContextSpec {
                base_commit_id: initial.head_commit_id.clone(),
                target_block_id: initial_block.id.clone(),
                target_block_revision: initial_block.revision,
                target_block_hash: initial_block.content_hash.clone(),
            })
            .unwrap();
        assert_eq!(document_only.len(), 1);
        assert_eq!(document_only[0].reason_codes, ["CURRENT_DOCUMENT_SUMMARY"]);

        let final_batch = project
            .refresh_summaries(&RefreshSummariesSpec { max_items: 8 })
            .unwrap();
        assert_eq!(final_batch.processed, 1);
        assert_eq!(final_batch.remaining, 0);
        let complete = project
            .summary_context(&SummaryContextSpec {
                base_commit_id: initial.head_commit_id.clone(),
                target_block_id: initial_block.id.clone(),
                target_block_revision: initial_block.revision,
                target_block_hash: initial_block.content_hash.clone(),
            })
            .unwrap();
        assert_eq!(complete.len(), 2);
        assert!(
            complete
                .iter()
                .all(|item| item.source_hash.starts_with("sha256:"))
        );
        assert!(
            complete
                .iter()
                .any(|item| item.reason_codes == ["PROJECT_SUMMARY"])
        );

        let saved = project
            .save_block(&SaveBlockSpec {
                block_id: initial_block.id.clone(),
                expected_revision: initial_block.revision,
                expected_hash: initial_block.content_hash.clone(),
                content: serde_json::json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "更新后的正文" }]
                }),
                plain_text: "更新后的正文".into(),
            })
            .unwrap();
        assert!(matches!(
            project.summary_context(&SummaryContextSpec {
                base_commit_id: initial.head_commit_id,
                target_block_id: initial_block.id,
                target_block_revision: initial_block.revision,
                target_block_hash: initial_block.content_hash,
            }),
            Err(WorkspaceCommandError::Validation(_))
        ));
        let stale_summaries = project
            .summary_context(&SummaryContextSpec {
                base_commit_id: saved.head_commit_id.clone(),
                target_block_id: saved.block.id.clone(),
                target_block_revision: saved.block.revision,
                target_block_hash: saved.block.content_hash.clone(),
            })
            .unwrap();
        assert!(stale_summaries.is_empty());
        project
            .refresh_summaries(&RefreshSummariesSpec { max_items: 8 })
            .unwrap();
        let refreshed = project
            .summary_context(&SummaryContextSpec {
                base_commit_id: saved.head_commit_id,
                target_block_id: saved.block.id,
                target_block_revision: saved.block.revision,
                target_block_hash: saved.block.content_hash,
            })
            .unwrap();
        assert_eq!(refreshed.len(), 2);
        assert!(
            refreshed
                .iter()
                .all(|item| item.content.contains("更新后的正文"))
        );
    }

    #[test]
    fn maintains_canonical_knowledge_and_serves_only_current_target_bound_context() {
        let parent = TempParent::new();
        let mut project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let workspace = project.workspace().unwrap();
        let target = &workspace.blocks[0];
        let fact = project
            .create_knowledge_item(&CreateKnowledgeItemSpec {
                kind: "fact".into(),
                title: "主角视觉".into(),
                content: "主角左眼失明".into(),
                sensitivity: "local_sensitive".into(),
                severity: None,
            })
            .unwrap();
        let constraint = project
            .create_knowledge_item(&CreateKnowledgeItemSpec {
                kind: "constraint".into(),
                title: "禁止剧透".into(),
                content: "本章不得揭示凶手身份".into(),
                sensitivity: "never_send".into(),
                severity: Some("hard".into()),
            })
            .unwrap();
        assert_eq!(project.knowledge_items().unwrap().len(), 2);

        let context = project
            .knowledge_context(&KnowledgeContextSpec {
                base_commit_id: workspace.head_commit_id.clone(),
                target_block_id: target.id.clone(),
                target_block_revision: target.revision,
                target_block_hash: target.content_hash.clone(),
            })
            .unwrap();
        assert_eq!(context.len(), 2);
        assert!(context.iter().all(|item| item.tier == "L3_KNOWLEDGE"));
        assert!(context.iter().all(|item| item.render_mode == "constraint"));
        assert!(
            context
                .iter()
                .all(|item| item.authority == "user_confirmed")
        );
        assert!(context.iter().any(|item| {
            item.reason_codes == ["CANONICAL_FACT"]
                && item.content == "事实【主角视觉】：主角左眼失明"
        }));
        assert!(context.iter().any(|item| {
            item.reason_codes == ["PROJECT_HARD_CONSTRAINT"] && item.sensitivity == "never_send"
        }));

        project
            .set_knowledge_item_status(&SetKnowledgeItemStatusSpec {
                id: fact.id,
                expected_revision: fact.revision,
                status: "archived".into(),
            })
            .unwrap();
        project
            .set_knowledge_item_status(&SetKnowledgeItemStatusSpec {
                id: constraint.id,
                expected_revision: constraint.revision,
                status: "rejected".into(),
            })
            .unwrap();
        assert!(
            project
                .knowledge_context(&KnowledgeContextSpec {
                    base_commit_id: workspace.head_commit_id.clone(),
                    target_block_id: target.id.clone(),
                    target_block_revision: target.revision,
                    target_block_hash: target.content_hash.clone(),
                })
                .unwrap()
                .is_empty()
        );
        assert!(matches!(
            project.knowledge_context(&KnowledgeContextSpec {
                base_commit_id: "commit-stale".into(),
                target_block_id: target.id.clone(),
                target_block_revision: target.revision,
                target_block_hash: target.content_hash.clone(),
            }),
            Err(WorkspaceCommandError::KnowledgeValidation(_))
        ));
    }

    #[test]
    fn collects_all_operation_context_sources_from_the_current_host_snapshot() {
        let parent = TempParent::new();
        let mut project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let initial = project.workspace().unwrap();
        let block = &initial.blocks[0];
        let saved = project
            .save_block(&SaveBlockSpec {
                block_id: block.id.clone(),
                expected_revision: block.revision,
                expected_hash: block.content_hash.clone(),
                content: serde_json::json!({
                    "type": "paragraph",
                    "content": [{ "type": "text", "text": "开场😀雨停了。" }]
                }),
                plain_text: "开场😀雨停了。".into(),
            })
            .unwrap();
        let style = project
            .create_style_sample(&CreateStyleSampleSpec {
                title: "短句".into(),
                content: "灯还亮着。".into(),
                sensitivity: "local_sensitive".into(),
            })
            .unwrap();
        project
            .set_style_sample_status(&SetStyleSampleStatusSpec {
                id: style.id,
                expected_revision: style.revision,
                status: "archived".into(),
            })
            .unwrap();
        project
            .create_knowledge_item(&CreateKnowledgeItemSpec {
                kind: "fact".into(),
                title: "天气".into(),
                content: "雨已经停了".into(),
                sensitivity: "local_sensitive".into(),
                severity: None,
            })
            .unwrap();
        project
            .refresh_summaries(&RefreshSummariesSpec { max_items: 8 })
            .unwrap();

        let spec = OperationContextSpec {
            base_commit_id: saved.head_commit_id.clone(),
            target_block_id: saved.block.id.clone(),
            target_block_revision: saved.block.revision,
            target_block_hash: saved.block.content_hash.clone(),
            from: 2,
            to: 4,
        };
        let candidates = project.operation_context(&spec).unwrap();
        assert!(candidates.iter().all(|item| {
            item.source_commit_id == saved.head_commit_id && item.source_hash.starts_with("sha256:")
        }));
        let target = candidates
            .iter()
            .find(|item| item.tier == "L0_TARGET")
            .unwrap();
        assert_eq!(target.content, "😀");
        assert!(target.mandatory && target.selected_by_user);
        assert!(
            candidates
                .iter()
                .any(|item| { item.reason_codes == ["TARGET_PREFIX"] && item.content == "开场" })
        );
        assert!(candidates.iter().any(|item| {
            item.reason_codes == ["TARGET_SUFFIX"] && item.content == "雨停了。"
        }));
        assert!(candidates.iter().any(|item| {
            item.reason_codes == ["DOCUMENT_STRUCTURE"] && item.content.contains("My Novel")
        }));
        assert!(candidates.iter().any(|item| {
            item.source_ref.starts_with("summary:project:") && item.tier == "L3_KNOWLEDGE"
        }));
        assert!(candidates.iter().any(|item| {
            item.source_ref.starts_with("knowledge:fact:") && item.status == "canonical"
        }));
        assert!(
            candidates
                .iter()
                .any(|item| { item.source_ref.starts_with("style:") && item.status == "archived" })
        );

        let mut split_surrogate = spec.clone();
        split_surrogate.from = 3;
        assert!(matches!(
            project.operation_context(&split_surrogate),
            Err(WorkspaceCommandError::ContextValidation(_))
        ));
        let mut stale = spec;
        stale.base_commit_id = "commit-stale".into();
        assert!(matches!(
            project.operation_context(&stale),
            Err(WorkspaceCommandError::ContextValidation(_))
        ));
    }

    #[test]
    fn versions_document_lifecycle_and_restores_it_from_a_checkpoint() {
        let parent = TempParent::new();
        let mut project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let initial = project.workspace().unwrap();
        let first = initial.documents[0].clone();
        let created = project
            .create_document(&CreateDocumentSpec {
                title: "第二章".into(),
                initial_text: "雨落在旧站台。".into(),
                parent_id: None,
            })
            .unwrap();
        let checkpoint = project.create_checkpoint().unwrap();
        let checkpoint_root = checkpoint.root_hash.clone();
        let before_block = project
            .workspace()
            .unwrap()
            .blocks
            .into_iter()
            .find(|block| block.document_id == created.document.id)
            .unwrap();

        let renamed = project
            .rename_document(&RenameDocumentSpec {
                document_id: created.document.id.clone(),
                expected_revision: created.document.revision,
                title: "第二章：雨夜".into(),
            })
            .unwrap();
        assert_eq!(renamed.workspace.documents[1].title, "第二章：雨夜");
        let rename_commit = project
            .version_history()
            .unwrap()
            .commits
            .into_iter()
            .find(|commit| commit.id == renamed.commit_id)
            .unwrap();
        assert_ne!(rename_commit.root_hash, checkpoint_root);
        assert_eq!(
            renamed
                .workspace
                .blocks
                .iter()
                .find(|block| block.id == before_block.id)
                .unwrap()
                .content_hash,
            before_block.content_hash
        );

        let renamed_document = renamed
            .workspace
            .documents
            .iter()
            .find(|document| document.id == created.document.id)
            .unwrap();
        let reordered = project
            .reorder_document(&ReorderDocumentSpec {
                document_id: renamed_document.id.clone(),
                expected_revision: renamed_document.revision,
                direction: crate::DocumentMoveDirection::Up,
            })
            .unwrap();
        assert_eq!(reordered.workspace.documents[0].id, created.document.id);
        let reordered_document = reordered
            .workspace
            .documents
            .iter()
            .find(|document| document.id == created.document.id)
            .unwrap();
        let archived = project
            .set_document_archived(&SetDocumentArchivedSpec {
                document_id: reordered_document.id.clone(),
                expected_revision: reordered_document.revision,
                archived: true,
            })
            .unwrap();
        assert_eq!(archived.workspace.documents.len(), 1);
        assert_eq!(archived.workspace.documents[0].id, first.id);
        assert_eq!(
            project.archived_documents().unwrap()[0].title,
            "第二章：雨夜"
        );

        let restored = project
            .restore_checkpoint(&RestoreCheckpointSpec {
                checkpoint_id: checkpoint.id,
            })
            .unwrap();
        assert_eq!(restored.restored_root_hash, checkpoint_root);
        assert_eq!(restored.workspace.documents.len(), 2);
        assert_eq!(restored.workspace.documents[0].id, first.id);
        assert_eq!(restored.workspace.documents[1].id, created.document.id);
        assert_eq!(restored.workspace.documents[1].title, "第二章");
        assert!(project.archived_documents().unwrap().is_empty());
        assert_eq!(restored.workspace.blocks.len(), 2);
    }

    #[test]
    fn versions_nested_document_reparenting_sibling_order_and_subtree_archive() {
        let parent = TempParent::new();
        let mut project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let first = project.workspace().unwrap().documents[0].clone();
        let second = project
            .create_document(&CreateDocumentSpec {
                title: "第二章".into(),
                initial_text: "第二章正文".into(),
                parent_id: None,
            })
            .unwrap()
            .document;
        let indented = project
            .change_document_depth(&ChangeDocumentDepthSpec {
                document_id: second.id.clone(),
                expected_revision: second.revision,
                direction: crate::DocumentDepthDirection::Indent,
            })
            .unwrap();
        let second = indented
            .workspace
            .documents
            .iter()
            .find(|document| document.id == second.id)
            .unwrap()
            .clone();
        assert_eq!(second.parent_id.as_deref(), Some(first.id.as_str()));
        assert!(project.summary_invalidations().unwrap().iter().any(|item| {
            item.scope_type == "document"
                && item.scope_id == first.id
                && item.source_commit_id == indented.commit_id
        }));

        let third = project
            .create_document(&CreateDocumentSpec {
                title: "场景三".into(),
                initial_text: "场景正文".into(),
                parent_id: Some(first.id.clone()),
            })
            .unwrap()
            .document;
        let reordered = project
            .reorder_document(&ReorderDocumentSpec {
                document_id: third.id.clone(),
                expected_revision: third.revision,
                direction: crate::DocumentMoveDirection::Up,
            })
            .unwrap();
        let child_ids = reordered
            .workspace
            .documents
            .iter()
            .filter(|document| document.parent_id.as_deref() == Some(first.id.as_str()))
            .map(|document| document.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(child_ids, vec![third.id.as_str(), second.id.as_str()]);
        let third = reordered
            .workspace
            .documents
            .iter()
            .find(|document| document.id == third.id)
            .unwrap()
            .clone();
        let outdented = project
            .change_document_depth(&ChangeDocumentDepthSpec {
                document_id: third.id.clone(),
                expected_revision: third.revision,
                direction: crate::DocumentDepthDirection::Outdent,
            })
            .unwrap();
        let first = outdented
            .workspace
            .documents
            .iter()
            .find(|document| document.id == first.id)
            .unwrap()
            .clone();
        let third = outdented
            .workspace
            .documents
            .iter()
            .find(|document| document.id == third.id)
            .unwrap()
            .clone();
        assert_eq!(third.parent_id, None);
        assert!(project.summary_invalidations().unwrap().iter().any(|item| {
            item.scope_type == "document"
                && item.scope_id == first.id
                && item.source_commit_id == outdented.commit_id
        }));

        let archived = project
            .set_document_archived(&SetDocumentArchivedSpec {
                document_id: first.id.clone(),
                expected_revision: first.revision,
                archived: true,
            })
            .unwrap();
        assert_eq!(archived.workspace.documents.len(), 1);
        assert_eq!(archived.workspace.documents[0].id, third.id);
        let archived_documents = project.archived_documents().unwrap();
        assert_eq!(archived_documents.len(), 2);
        let archived_first = archived_documents
            .iter()
            .find(|document| document.id == first.id)
            .unwrap();
        let archived_second = archived_documents
            .iter()
            .find(|document| document.id == second.id)
            .unwrap();
        assert!(matches!(
            project.set_document_archived(&SetDocumentArchivedSpec {
                document_id: archived_second.id.clone(),
                expected_revision: archived_second.revision,
                archived: false,
            }),
            Err(WorkspaceCommandError::DocumentValidation(_))
        ));
        project
            .set_document_archived(&SetDocumentArchivedSpec {
                document_id: archived_first.id.clone(),
                expected_revision: archived_first.revision,
                archived: false,
            })
            .unwrap();
        let archived_second = project
            .archived_documents()
            .unwrap()
            .into_iter()
            .find(|document| document.id == second.id)
            .unwrap();
        let restored = project
            .set_document_archived(&SetDocumentArchivedSpec {
                document_id: archived_second.id,
                expected_revision: archived_second.revision,
                archived: false,
            })
            .unwrap();
        assert_eq!(restored.workspace.documents.len(), 3);
        assert!(
            project
                .version_history()
                .unwrap()
                .commits
                .iter()
                .any(|commit| commit.reason == "document_reparent")
        );
    }

    #[test]
    fn atomically_applies_a_reviewed_proposal_as_an_ai_accept_commit() {
        let parent = TempParent::new();
        let mut project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let workspace = project.workspace().unwrap();
        let block = &workspace.blocks[0];
        let run_id = "run-apply-test";
        let intent_id = "intent-apply-test";
        let proposal_id = "proposal-apply-test";
        let created_at = "2026-07-15T00:00:00Z";
        let mut proposal = serde_json::json!({
            "schemaVersion": 2,
            "id": proposal_id,
            "operationRunId": run_id,
            "baseCommitId": workspace.head_commit_id,
            "target": {
                "documentId": block.document_id,
                "blockId": block.id,
                "baseRevision": block.revision,
                "baseHash": block.content_hash,
                "from": { "blockId": block.id, "offset": 0 },
                "to": { "blockId": block.id, "offset": 0 }
            },
            "hunks": [{
                "id": "proposal-apply-test:h1",
                "from": { "blockId": block.id, "offset": 0, "affinity": "after" },
                "to": { "blockId": block.id, "offset": 0, "affinity": "before" },
                "original": "",
                "replacement": "你好，世界",
                "granularity": "token"
            }],
            "warnings": [],
            "status": "review",
            "createdAt": created_at
        });
        let proposal_hash = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(&proposal).unwrap())
        );
        proposal["proposalHash"] = serde_json::json!(proposal_hash);
        let transitions = [
            ("draft", "compiling"),
            ("compiling", "preflight"),
            ("preflight", "queued"),
            ("queued", "streaming"),
            ("streaming", "validating"),
            ("validating", "review"),
        ]
        .map(|(from, to)| {
            serde_json::json!({
                "fromState": from,
                "toState": to,
                "occurredAt": created_at
            })
        });
        let bundle = serde_json::json!({
            "schemaVersion": 1,
            "run": {
                "id": run_id,
                "operationIntentId": intent_id,
                "projectId": workspace.project_id,
                "baseCommitId": workspace.head_commit_id,
                "providerId": "deepseek",
                "model": "deepseek-v4-flash",
                "state": "review",
                "responseId": "response-apply-test",
                "finishReason": "stop",
                "startedAt": created_at,
                "updatedAt": created_at
            },
            "contextPacket": {
                "id": "context-apply-test",
                "operationIntentId": intent_id,
                "projectId": workspace.project_id,
                "baseCommitId": workspace.head_commit_id,
                "packetHash": "sha256:context-apply-test",
                "payload": { "schemaVersion": 1, "kind": "test" },
                "createdAt": created_at
            },
            "lifecycleEvents": transitions,
            "artifact": {
                "id": proposal_id,
                "kind": "patch_proposal",
                "bindingHash": proposal_hash,
                "payload": proposal,
                "createdAt": created_at
            }
        });
        project
            .operations_mut()
            .persist_operation_bundle_json(&bundle.to_string())
            .unwrap();
        project
            .operations_mut()
            .append_review_event_json(
                &serde_json::json!({
                    "schemaVersion": 1,
                    "id": "review-decision-apply-test",
                    "proposalId": proposal_id,
                    "expectedRevision": 0,
                    "expectedStatus": "review",
                    "kind": "decision",
                    "nextStatus": "ready",
                    "hunkId": "proposal-apply-test:h1",
                    "decision": "accepted",
                    "occurredAt": created_at
                })
                .to_string(),
            )
            .unwrap();

        let candidates = project.review_candidates().unwrap();
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].proposal_id, proposal_id);
        assert_eq!(candidates[0].status, "ready");
        let detail = project.review_candidate(proposal_id).unwrap();
        assert_eq!(detail.session.revision, 1);
        assert_eq!(
            detail.session.decisions.get("proposal-apply-test:h1"),
            Some(&"accepted".into())
        );
        let branch = project
            .create_review_candidate_branch(&CreateReviewCandidateBranchSpec {
                proposal_id: proposal_id.into(),
                expected_review_revision: 1,
                branch_name: "AI 候选：车站续写".into(),
            })
            .unwrap();
        assert_eq!(branch.main_head_commit_id, workspace.head_commit_id);
        assert_eq!(project.workspace().unwrap(), workspace);
        assert!(
            project
                .version_history()
                .unwrap()
                .checkpoints
                .iter()
                .all(|checkpoint| checkpoint.id != branch.branch.snapshot_id)
        );
        assert_eq!(
            project
                .review_candidates()
                .unwrap()
                .first()
                .and_then(|candidate| candidate.candidate_branch.as_ref())
                .map(|candidate| candidate.branch_name.as_str()),
            Some("AI 候选：车站续写")
        );

        let applied = project
            .apply_reviewed_proposal(&ApplyReviewedProposalSpec {
                proposal_id: proposal_id.into(),
                expected_review_revision: 1,
            })
            .unwrap();
        assert_eq!(applied.accepted_hunks, 1);
        assert_eq!(applied.rejected_hunks, 0);
        assert_eq!(applied.review_revision, 2);
        assert_eq!(applied.save.block.plain_text, "你好，世界");
        let commit = project
            .version_history()
            .unwrap()
            .commits
            .into_iter()
            .find(|commit| commit.id == applied.save.commit_id)
            .unwrap();
        assert_eq!(commit.reason, "ai_accept");
        assert_eq!(commit.actor_type, "model");
        assert_eq!(commit.actor_id.as_deref(), Some(run_id));
        let audit = project.operations().get_operation_audit(run_id).unwrap();
        assert_eq!(audit.run.state, "accepted");
        assert_eq!(audit.review.unwrap().status, "applied");
    }

    #[test]
    fn rejects_unsafe_or_existing_package_names_without_staging_debris() {
        let parent = TempParent::new();
        for folder_name in [
            "../escape.optimizer",
            "CON.optimizer",
            "bad?.optimizer",
            "plain",
        ] {
            let mut invalid = spec();
            invalid.folder_name = folder_name.into();
            assert!(matches!(
                OpenedProject::create(&parent.0, &invalid),
                Err(ProjectPackageError::Validation(_))
            ));
        }
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        drop(project);
        assert!(matches!(
            OpenedProject::create(&parent.0, &spec()),
            Err(ProjectPackageError::AlreadyExists)
        ));
        assert_eq!(
            fs::read_dir(&parent.0)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry.file_name().to_string_lossy().contains(".creating-"))
                .count(),
            0
        );
    }

    #[test]
    fn rejects_manifest_tampering_before_exposing_the_store() {
        let parent = TempParent::new();
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let directory = project.info().unwrap().directory;
        drop(project);
        let manifest_path = Path::new(&directory).join(MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["projectId"] = serde_json::Value::String("project-tampered".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(matches!(
            OpenedProject::open(directory),
            Err(ProjectPackageError::Store(StoreError::NotFound { .. }))
        ));
    }

    #[test]
    fn rejects_a_main_branch_manifest_that_does_not_bind_to_the_project() {
        let parent = TempParent::new();
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let directory = project.info().unwrap().directory;
        drop(project);
        let manifest_path = Path::new(&directory).join(MANIFEST_FILE);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["mainBranchId"] = serde_json::Value::String("branch-tampered".into());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        assert!(matches!(
            OpenedProject::open(directory),
            Err(ProjectPackageError::Store(StoreError::NotFound { .. }))
        ));
    }

    #[test]
    fn export_diagnostics_writes_a_pretty_json_bundle_into_the_exports_directory() {
        let parent = TempParent::new();
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let info = project.info().unwrap();
        let response = project.export_diagnostics().unwrap();
        assert_eq!(response.schema_version, 1);
        let path = Path::new(&response.path);
        assert!(path.starts_with(Path::new(&info.directory).join("exports")));
        assert!(
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("optimizer-diagnostics-")
                    && name.ends_with(".json"))
        );
        assert!(path.is_file());
        let bytes = fs::read(path).unwrap();
        assert!(!bytes.is_empty());
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schemaVersion"], serde_json::json!(4));
        assert!(value["manifest"].is_object());
        assert!(value["insights"].is_object());
        assert!(value["recentRuns"].is_array());
    }

    #[test]
    fn export_diagnostics_manifest_binds_to_the_current_project_and_store_schema() {
        let parent = TempParent::new();
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let info = project.info().unwrap();
        let response = project.export_diagnostics().unwrap();
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(&response.path).unwrap()).unwrap();
        let manifest = &value["manifest"];
        assert_eq!(manifest["projectId"], serde_json::json!(info.project_id));
        assert_eq!(manifest["projectTitle"], serde_json::json!(info.title));
        assert_eq!(
            manifest["storeSchemaVersion"],
            serde_json::json!(info.database_schema_version as u32)
        );
        let generated_at = manifest["generatedAt"].as_str().unwrap();
        assert!(OffsetDateTime::parse(generated_at, &Rfc3339).is_ok());
    }

    #[test]
    fn export_diagnostics_bundle_round_trips_through_serde_json() {
        let parent = TempParent::new();
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let info = project.info().unwrap();
        let response = project.export_diagnostics().unwrap();
        let json = fs::read_to_string(&response.path).unwrap();
        let bundle: DiagnosticBundle = serde_json::from_str(&json).unwrap();
        assert_eq!(bundle.schema_version, 4);
        assert_eq!(bundle.manifest.project_id, info.project_id);
        assert_eq!(bundle.manifest.project_title, info.title);
        assert_eq!(
            bundle.manifest.store_schema_version,
            info.database_schema_version as u32
        );
        assert!(OffsetDateTime::parse(&bundle.manifest.generated_at, &Rfc3339).is_ok());
        let reserialized = serde_json::to_string(&bundle).unwrap();
        let reparsed: DiagnosticBundle = serde_json::from_str(&reserialized).unwrap();
        assert_eq!(bundle, reparsed);
    }

    #[test]
    fn export_diagnostics_includes_operation_insights_and_recent_runs() {
        let parent = TempParent::new();
        let project = OpenedProject::create(&parent.0, &spec()).unwrap();
        let response = project.export_diagnostics().unwrap();
        let json = fs::read_to_string(&response.path).unwrap();
        let bundle: DiagnosticBundle = serde_json::from_str(&json).unwrap();
        let insights = &bundle.insights;
        assert_eq!(insights.total_input_tokens, 0);
        assert_eq!(insights.total_output_tokens, 0);
        assert_eq!(insights.total_tokens, 0);
        assert_eq!(insights.accepted_count, 0);
        assert_eq!(insights.rejected_count, 0);
        assert_eq!(insights.conflicted_count, 0);
        assert_eq!(insights.total_runs, 0);
        assert!(bundle.recent_runs.is_empty());
    }

    #[test]
    fn t03_diagnostic_bundle_v3_carries_extended_recent_runs() {
        let run = RecentRunSummary {
            run_id: "run-1".into(),
            operation_intent_id: "intent-1".into(),
            state: "accepted".into(),
            provider_id: "deepseek".into(),
            started_at: "2026-07-15T00:00:00Z".into(),
            total_tokens: Some(1700),
            input_tokens: 1000,
            output_tokens: 500,
            cached_input_tokens: Some(200),
        };
        let bundle = DiagnosticBundle {
            schema_version: 3,
            manifest: DiagnosticManifest {
                project_id: "project-1".into(),
                project_title: "Test".into(),
                generated_at: "2026-07-15T00:00:00Z".into(),
                store_schema_version: 10,
            },
            insights: OperationInsightsRecord {
                total_input_tokens: 1000,
                total_output_tokens: 500,
                total_tokens: 1700,
                accepted_count: 1,
                rejected_count: 0,
                conflicted_count: 0,
                total_runs: 1,
            },
            recent_runs: vec![run],
            revision_metrics: RevisionMetrics::default(),
            payload_hash_variations: PayloadHashVariations::default(),
        };
        let json = serde_json::to_value(&bundle).unwrap();
        assert_eq!(json["schemaVersion"], serde_json::json!(3));
        let run_json = &json["recentRuns"][0];
        assert_eq!(run_json["inputTokens"], serde_json::json!(1000));
        assert_eq!(run_json["outputTokens"], serde_json::json!(500));
        assert_eq!(run_json["cachedInputTokens"], serde_json::json!(200));
        assert_eq!(run_json["totalTokens"], serde_json::json!(1700));
        let reparsed: DiagnosticBundle = serde_json::from_value(json).unwrap();
        assert_eq!(reparsed.schema_version, 3);
        assert_eq!(reparsed.recent_runs[0].input_tokens, 1000);
        assert_eq!(reparsed.recent_runs[0].output_tokens, 500);
        assert_eq!(reparsed.recent_runs[0].cached_input_tokens, Some(200));
    }
