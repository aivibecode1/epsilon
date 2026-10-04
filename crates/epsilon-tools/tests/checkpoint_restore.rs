//! Integration coverage for the **rollback half** of the checkpoint store.
//!
//! The unit tests beside the store drive [`CheckpointStore::restore`] with an
//! [`Entry`] they built themselves. These drive it the way production does — through
//! the `write` tool that records the checkpoint, and through [`CheckpointStore::resolve`],
//! which is what every front end calls — because the defect this file exists for was
//! never a wrong answer, it was **no caller at all**: a store that recorded correctly,
//! restored correctly under test, and could not be reached from any surface.
//!
//! So every test here goes through the public API only, and the first one asserts the
//! loop closes: a real `write` records, a real `resolve` finds it, a real `restore`
//! puts the original bytes back.

use std::path::Path;
use std::sync::Arc;

use epsilon_contracts::tool::{Tool, ToolContext, ToolInvocation};
use epsilon_tools::checkpoint::CheckpointStore;
use epsilon_tools::{ReadTool, WriteTool};
use tokio_util::sync::CancellationToken;

fn ctx_for(root: &Path) -> ToolContext {
    ToolContext {
        session_id: "test".into(),
        observation_scope: Arc::new(()),
        cwd: root.to_path_buf(),
        spill_dir: root.join(".spill"),
        token: CancellationToken::new(),
        task_host: None,
        goal_host: None,
    }
}

/// Run one tool call and return its text, as `builtin_tools.rs` does.
async fn run(
    tool: &dyn Tool,
    ctx: &ToolContext,
    args: serde_json::Value,
) -> Result<String, String> {
    let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
    tool.execute(
        ToolInvocation {
            tool_call_id: "tc_restore".into(),
            args,
        },
        ctx.clone(),
        sink,
    )
    .await
    .map(|o| {
        o.content
            .iter()
            .filter_map(|c| match c {
                epsilon_contracts::content::ContentBlock::Text { text } => Some(text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    })
    .map_err(|f| f.message)
}

/// A workspace with one file, read and then overwritten through the **real** tools —
/// `write` is the only production writer of checkpoints (`files.rs`) — and the store it
/// wrote into.
///
/// The read is not decoration. `write` refuses to overwrite a file this session has not
/// read (`read_log`), which is the harness refusing to replace work nobody has seen; a
/// test that wrote over a seeded file without reading it would have been asserting on
/// the refusal rather than on the checkpoint.
async fn workspace_after_a_write() -> (tempfile::TempDir, ToolContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/lib.rs"), "the original\n").unwrap();
    let ctx = ctx_for(root);
    run(
        &ReadTool::new(),
        &ctx,
        serde_json::json!({ "path": "src/lib.rs" }),
    )
    .await
    .expect("the read lands");
    let text = run(
        &WriteTool::new(),
        &ctx,
        serde_json::json!({ "path": "src/lib.rs", "content": "what the model wrote\n" }),
    )
    .await
    .expect("the write lands");
    assert!(text.contains("overwrote"), "{text}");
    (dir, ctx)
}

/// The loop that is reachable end to end today: a real `write` **creates** a file, and
/// reverting its creation removes it.
///
/// This is the success path, and it is asserted on the file rather than on a return
/// value, because "the checkpoint is reachable" is a claim about bytes on disk. Before
/// the rollback half was wired to a command it had callers only inside its own module,
/// and no public way for a front end to turn what a reader typed into an entry existed
/// at all — so this test could not have been written.
#[tokio::test]
async fn a_file_the_agent_created_is_removed_by_restore() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let ctx = ctx_for(root);
    let text = run(
        &WriteTool::new(),
        &ctx,
        serde_json::json!({ "path": "src/new.rs", "content": "// written by the model\n" }),
    )
    .await
    .expect("the write lands");
    assert!(text.contains("created"), "{text}");
    assert!(root.join("src/new.rs").exists(), "the file exists");

    let store = CheckpointStore::open(root);
    let entry = store
        .resolve(root, "src/new.rs")
        .expect("the creation's checkpoint resolves");
    assert!(
        !entry.existed,
        "the checkpoint knows the file did not exist"
    );

    // 0 bytes means "the revert was a deletion", which is the only other shape a
    // restore has. There is no digest to compare on this shape — the file did not
    // exist when the entry was taken, so there is no pre-image to hash — which is why
    // `/restore` previews a deletion and waits for a typed `yes` rather than acting on
    // the name alone.
    assert_eq!(
        store.restore(root, &entry).expect("the revert is allowed"),
        0
    );
    assert!(
        !root.join("src/new.rs").exists(),
        "reverting a creation removes the file"
    );
}

/// **Reverting an edit works, and this test used to prove that it could not.**
///
/// # The defect, and why the shape of this test changed
///
/// This began as `reverting_an_edited_file_refuses_because_the_digest_is_the_preimage`,
/// and it pinned a real defect rather than asserting a behaviour anybody chose.
/// `Entry::digest` held the hash of the content the file had when the entry was captured
/// — the *pre-image* — and `restore` refused unless the file still hashed to it. But
/// `record` runs **before** the tool mutates the file, so by the time anyone reverted,
/// the file held the tool's output and the two hashes could never agree: **the guard was
/// unsatisfiable on the only flow that produces checkpoints**, and a content revert
/// refused 100% of the time. M104's own commit (`d4d70ff`) records why nobody noticed:
/// *"No UI reaches this yet, which is why it is a fix rather than an incident."* Wiring
/// `/restore` up is what made it observable.
///
/// The fix was **not** to weaken the guard. `Entry` now also carries `post_image` — the
/// hash of what the harness itself left on disk — written by
/// `CheckpointStore::amend_with_post_image` after the write lands. The guard now asks the
/// one question that is answerable: *is the file still exactly what this harness left
/// here?* Which means the test had to invert: the same workspace that used to prove a
/// revert impossible now proves it works, and the refusal case moved to
/// `a_file_changed_since_the_capture_is_refused_and_left_alone`, which is the half that
/// must keep refusing.
///
/// The pinned version is what made this fix honest — it would have failed loudly rather
/// than let a guard quietly stop guarding — so the record of it belongs here even though
/// the assertion now points the other way.
#[tokio::test]
async fn reverting_an_edited_file_puts_the_pre_image_back() {
    let (dir, _ctx) = workspace_after_a_write().await;
    let root = dir.path();
    let store = CheckpointStore::open(root);
    let entry = store.resolve(root, "src/lib.rs").expect("resolves");

    // The file still holds what the model wrote — nobody touched it since.
    assert_eq!(
        std::fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        "what the model wrote\n"
    );

    let restored = store
        .restore(root, &entry)
        .expect("an untouched edit must be revertible — this is the whole point of the command");
    assert_eq!(restored, "the original\n".len() as u64, "bytes put back");
    assert_eq!(
        std::fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        "the original\n",
        "the file holds the pre-image again"
    );
}

/// A file the user changed since the capture is **not** overwritten, and the refusal
/// leaves it exactly as it is.
///
/// The failure this pins is the worst one a revert path can produce: the user edits a
/// file after the agent's snapshot, asks for an undo, and loses their own work. The
/// snapshot can now tell their edit from the harness's own (see
/// `reverting_an_edited_file_puts_the_pre_image_back`), which is the only reason it is
/// able to refuse here — and the refusal is only worth having if the file survives it.
#[tokio::test]
async fn a_file_changed_since_the_capture_is_refused_and_left_alone() {
    let (dir, _ctx) = workspace_after_a_write().await;
    let root = dir.path();
    std::fs::write(root.join("src/lib.rs"), "the user's later work\n").unwrap();

    let store = CheckpointStore::open(root);
    let entry = store.resolve(root, "src/lib.rs").expect("resolves");

    let why = store
        .restore(root, &entry)
        .expect_err("a changed file must not be overwritten");
    assert!(
        why.contains("src/lib.rs"),
        "the refusal names the file: {why}"
    );
    assert!(
        why.contains("changed after this harness wrote it"),
        "the refusal says why: {why}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        "the user's later work\n",
        "the refusal must leave the file exactly as it was"
    );
}

/// A path that escapes the workspace is refused, and names itself as the reason.
///
/// The ledger's paths are workspace-relative and [`CheckpointStore::restore`] rejoins
/// them onto the workspace, so a `..` that survived resolution would let a typed
/// argument name a file outside the tree this store owns. The refusal is checked before
/// the lookup rather than after, so a `../` never even reaches the ledger.
#[tokio::test]
async fn a_path_outside_the_workspace_is_refused_by_resolve() {
    let (dir, _ctx) = workspace_after_a_write().await;
    let root = dir.path();

    let store = CheckpointStore::open(root);
    let why = store
        .resolve(root, "../elsewhere.rs")
        .expect_err("an escape must not resolve to a checkpoint");
    assert!(
        why.contains("outside the workspace"),
        "the refusal says why: {why}"
    );
    // And the escaping spelling of the file that *does* have one resolves to the same
    // entry as the honest one, rather than to nothing.
    assert_eq!(
        store.resolve(root, "src/../src/lib.rs").map(|e| e.path),
        Ok("src/lib.rs".to_string()),
        "`..` inside the workspace is normalised, not refused"
    );
}

/// One spelling of a path, three ways a reader types it.
///
/// The doc comment on [`CheckpointStore::resolve`] claims a caller cannot miss by
/// guessing a second spelling of a ledger key. That is a claim about behaviour, so it
/// is checked here against all three spellings rather than asserted in prose: relative,
/// absolute, and Windows-separated. A caller that re-derived the key itself would fail
/// two of the three, and would report "no checkpoint" for a file that has one.
#[tokio::test]
async fn resolve_accepts_every_spelling_a_reader_types() {
    let (dir, _ctx) = workspace_after_a_write().await;
    let root = dir.path();
    let store = CheckpointStore::open(root);

    for spelling in [
        "src/lib.rs".to_string(),
        root.join("src").join("lib.rs").display().to_string(),
        "src\\lib.rs".to_string(),
        "  src/lib.rs  ".to_string(),
    ] {
        let entry = store
            .resolve(root, &spelling)
            .unwrap_or_else(|e| panic!("{spelling:?} must resolve: {e}"));
        assert_eq!(entry.path, "src/lib.rs", "for spelling {spelling:?}");
    }
}

/// An argument that names nothing says so — and names what *is* recoverable.
///
/// The empty case and the unknown-path case are separate sentences because they are
/// different facts: nothing has ever been checkpointed here, or this particular name is
/// not one of the ones that was. Collapsing them into one "not found" is what makes a
/// reader stop believing the messages that are true.
#[tokio::test]
async fn an_argument_that_names_nothing_says_so_and_lists_what_does() {
    let (dir, _ctx) = workspace_after_a_write().await;
    let root = dir.path();
    let store = CheckpointStore::open(root);

    let unknown = store
        .resolve(root, "src/parser.rs")
        .expect_err("an unrecorded name must not resolve");
    assert!(
        unknown.contains("no checkpoint for 'src/parser.rs'"),
        "the refusal quotes what was asked for: {unknown}"
    );
    assert!(
        unknown.contains("src/lib.rs"),
        "and names the file that does have one: {unknown}"
    );

    let empty = tempfile::tempdir().unwrap();
    let bare = CheckpointStore::open(empty.path());
    // Two different refusals for two different facts, checked separately: nothing was
    // ever captured here, and this particular name is not one of the ones that was.
    let never = bare
        .resolve(empty.path(), "src/lib.rs")
        .expect_err("an empty store has nothing for any name");
    assert!(
        never.contains("nothing in this workspace has one"),
        "{never}"
    );
    let why = bare
        .resolve(empty.path(), "   ")
        .expect_err("an empty argument must not resolve");
    assert!(why.contains("no file named"), "{why}");
}
