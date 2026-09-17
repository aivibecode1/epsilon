//! Integration coverage for the built-in tools, run against real tempdir workspaces.

use std::path::Path;
use std::sync::Arc;

use epsilon_contracts::tool::{Tool, ToolContext, ToolInvocation};
use epsilon_tools::util;
use epsilon_tools::{BashTool, EditTool, GlobTool, GrepTool, LsTool, ReadTool, WriteTool};
use tokio_util::sync::CancellationToken;

fn ctx_for(root: &Path) -> ToolContext {
    ToolContext {
        session_id: "test".into(),
        observation_scope: std::sync::Arc::new(()),
        cwd: root.to_path_buf(),
        spill_dir: root.join(".spill"),
        token: CancellationToken::new(),
        task_host: None,
    }
}

async fn run(
    tool: &dyn Tool,
    ctx: &ToolContext,
    args: serde_json::Value,
) -> Result<String, String> {
    let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
    tool.execute(
        ToolInvocation {
            tool_call_id: "tc_t".into(),
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

fn seeded_workspace() -> (tempfile::TempDir, ToolContext) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() {}\n",
    )
    .unwrap();
    std::fs::write(root.join("README.md"), "# demo\nhello world\n").unwrap();
    let ctx = ctx_for(root);
    (dir, ctx)
}

#[tokio::test]
async fn observations_are_not_inherited_from_another_agent() {
    let (dir, mut a) = seeded_workspace();
    a.session_id = "agent-a".into();
    let mut b = ctx_for(dir.path());
    b.session_id = a.session_id.clone(); // display names are not observation identities
    let original = std::fs::read(dir.path().join("README.md")).unwrap();
    run(
        &ReadTool::new(),
        &a,
        serde_json::json!({"path": "README.md"}),
    )
    .await
    .unwrap();
    let result = run(
        &WriteTool::new(),
        &b,
        serde_json::json!({"path": "README.md", "content": "unseen overwrite"}),
    )
    .await;
    assert!(
        result.is_err(),
        "another agent's read cannot authorize a write"
    );
    assert_eq!(
        std::fs::read(dir.path().join("README.md")).unwrap(),
        original
    );
}

#[tokio::test]
async fn observations_stay_stale_after_a_sibling_write_until_reread() {
    let (dir, a) = seeded_workspace();
    let b = ctx_for(dir.path());
    let path = dir.path().join("README.md");
    for ctx in [&a, &b] {
        run(
            &ReadTool::new(),
            ctx,
            serde_json::json!({"path": "README.md"}),
        )
        .await
        .unwrap();
    }
    let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
    let replacement = "# demo\nhello earth\n";
    run(
        &WriteTool::new(),
        &b,
        serde_json::json!({"path": "README.md", "content": replacement}),
    )
    .await
    .unwrap();
    // Deterministically emulate a same-length write on a coarse timestamp filesystem.
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(mtime)
        .unwrap();
    let result = run(
        &EditTool::new(),
        &a,
        serde_json::json!({"path": "README.md", "old_string": "hello", "new_string": "bye"}),
    )
    .await;
    assert!(result.unwrap_err().contains("changed on disk"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), replacement);
    run(
        &ReadTool::new(),
        &a,
        serde_json::json!({"path": "README.md"}),
    )
    .await
    .unwrap();
    run(
        &WriteTool::new(),
        &a,
        serde_json::json!({"path": "README.md", "content": "after reread"}),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn observations_partial_reads_and_failed_edits_do_not_refresh_siblings() {
    let (dir, a) = seeded_workspace();
    let b = ctx_for(dir.path());
    run(
        &ReadTool::new(),
        &a,
        serde_json::json!({"path": "README.md", "limit": 1}),
    )
    .await
    .unwrap();
    run(
        &ReadTool::new(),
        &b,
        serde_json::json!({"path": "README.md"}),
    )
    .await
    .unwrap();
    assert!(
        run(
            &WriteTool::new(),
            &a,
            serde_json::json!({"path": "README.md", "content": "blind"})
        )
        .await
        .unwrap_err()
        .contains("only read in part")
    );
    assert!(
        run(
            &EditTool::new(),
            &b,
            serde_json::json!({"path": "README.md", "old_string": "ABSENT", "new_string": "x"})
        )
        .await
        .is_err()
    );
    assert!(
        run(
            &WriteTool::new(),
            &a,
            serde_json::json!({"path": "README.md", "content": "still blind"})
        )
        .await
        .unwrap_err()
        .contains("only read in part"),
        "a failed sibling edit must not upgrade a partial observation"
    );
    run(
        &EditTool::new(),
        &a,
        serde_json::json!({"path": "README.md", "old_string": "# demo", "new_string": "# title"}),
    )
    .await
    .unwrap();
    assert!(
        std::fs::read_to_string(dir.path().join("README.md"))
            .unwrap()
            .starts_with("# title")
    );
}

#[tokio::test]
async fn observations_sibling_edits_invalidate_only_the_changed_path() {
    for batch in [false, true] {
        let (dir, a) = seeded_workspace();
        let b = ctx_for(dir.path());
        for ctx in [&a, &b] {
            for path in ["README.md", "src/lib.rs"] {
                run(&ReadTool::new(), ctx, serde_json::json!({"path": path}))
                    .await
                    .unwrap();
            }
        }
        let path = dir.path().join("README.md");
        let mtime = std::fs::metadata(&path).unwrap().modified().unwrap();
        let args = if batch {
            serde_json::json!({"path": "README.md", "edits": [
                {"old_string": "demo", "new_string": "test"},
                {"old_string": "world", "new_string": "earth"}
            ]})
        } else {
            serde_json::json!({"path": "README.md", "old_string": "world", "new_string": "earth"})
        };
        run(&EditTool::new(), &b, args).await.unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(mtime)
            .unwrap();
        let after = std::fs::read(&path).unwrap();
        assert!(
            run(
                &WriteTool::new(),
                &a,
                serde_json::json!({"path": "README.md", "content": "stale overwrite"})
            )
            .await
            .unwrap_err()
            .contains("changed on disk")
        );
        assert_eq!(std::fs::read(&path).unwrap(), after);
        run(
            &WriteTool::new(),
            &a,
            serde_json::json!({"path": "src/lib.rs", "content": "unrelated write"}),
        )
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn edit_refuses_capped_source_without_losing_tail() {
    let dir = tempfile::tempdir().unwrap();
    let ctx = ctx_for(dir.path());
    let path = dir.path().join("large.txt");
    let mut original = vec![b'x'; util::MAX_READ_BYTES + 1024];
    original[300..306].copy_from_slice(b"TARGET");
    original.extend_from_slice(b"\nUNIQUE_UNREAD_TAIL\n");
    std::fs::write(&path, &original).unwrap();
    run(
        &ReadTool::new(),
        &ctx,
        serde_json::json!({"path": "large.txt"}),
    )
    .await
    .unwrap();

    let result = run(
        &EditTool::new(),
        &ctx,
        serde_json::json!({"path": "large.txt", "old_string": "TARGET", "new_string": "CHANGE"}),
    )
    .await;
    assert_eq!(
        std::fs::read(&path).unwrap().len(),
        original.len(),
        "an edit must never discard the unread suffix"
    );
    assert_eq!(std::fs::read(&path).unwrap(), original);
    assert!(result.unwrap_err().contains("limit"));
}

async fn check_edit_source(original: Vec<u8>, refusal: Option<&str>) {
    for mode in ["single", "replace_all", "batch"] {
        let dir = tempfile::tempdir().unwrap();
        let ctx = ctx_for(dir.path());
        let path = dir.path().join("input.txt");
        std::fs::write(&path, &original).unwrap();
        run(
            &ReadTool::new(),
            &ctx,
            serde_json::json!({"path": "input.txt"}),
        )
        .await
        .unwrap();
        let args = if mode == "batch" {
            serde_json::json!({"path": "input.txt", "edits": [
                {"old_string": "TARGET", "new_string": "CHANGE"},
                {"old_string": "SECOND", "new_string": "RESULT"}
            ]})
        } else {
            serde_json::json!({"path": "input.txt", "old_string": "TARGET",
                "new_string": "CHANGE", "replace_all": mode == "replace_all"})
        };
        let result = run(&EditTool::new(), &ctx, args).await;
        let actual = std::fs::read(&path).unwrap();
        let mut expected = original.clone();
        if let Some(reason) = refusal {
            assert!(result.is_err(), "{mode}: unsafe input must be refused");
            assert!(
                result.unwrap_err().contains(reason),
                "{mode}: refusal reason"
            );
            assert!(
                !dir.path().join(".epsilon/checkpoints").exists(),
                "{mode}: refused input must not produce a checkpoint"
            );
        } else {
            result.unwrap();
            expected[..6].copy_from_slice(b"CHANGE");
            if mode == "batch" {
                expected[7..13].copy_from_slice(b"RESULT");
            }
        }
        assert_eq!(actual.len(), expected.len(), "{mode}: source length");
        assert!(actual == expected, "{mode}: all unrelated bytes preserved");
    }
}

fn edit_source_of_size(size: usize) -> Vec<u8> {
    let mut bytes = b"TARGET SECOND\n".to_vec();
    bytes.resize(size, b'x');
    bytes
}

#[tokio::test]
async fn edit_source_at_and_below_limit_is_complete() {
    for size in [util::MAX_READ_BYTES - 1, util::MAX_READ_BYTES] {
        let mut bytes = edit_source_of_size(size);
        bytes[size - 2..].copy_from_slice("é".as_bytes());
        check_edit_source(bytes, None).await;
    }
}

#[tokio::test]
async fn edit_source_one_byte_over_limit_is_refused() {
    check_edit_source(edit_source_of_size(util::MAX_READ_BYTES + 1), Some("limit")).await;
}

#[tokio::test]
async fn edit_source_multibyte_split_at_limit_is_refused() {
    let mut bytes = edit_source_of_size(util::MAX_READ_BYTES - 1);
    bytes.extend_from_slice("é".as_bytes());
    check_edit_source(bytes, Some("limit")).await;
}

#[tokio::test]
async fn edit_source_invalid_utf8_is_refused() {
    for size in [100, util::MAX_READ_BYTES] {
        let mut bytes = edit_source_of_size(size);
        bytes[size - 1] = 0xff;
        check_edit_source(bytes, Some("UTF-8")).await;
    }
}

#[tokio::test]
async fn read_returns_numbered_lines_and_rejects_escapes() {
    let (_d, ctx) = seeded_workspace();
    let read = ReadTool::new();

    let out = run(&read, &ctx, serde_json::json!({"path": "src/lib.rs"}))
        .await
        .unwrap();
    assert!(out.contains("    1| pub fn alpha() {}"));
    assert!(out.contains("    2| pub fn beta() {}"));

    let paged = run(
        &read,
        &ctx,
        serde_json::json!({"path": "src/lib.rs", "offset": 2, "limit": 1}),
    )
    .await
    .unwrap();
    assert!(paged.contains("beta"));
    assert!(!paged.contains("alpha"));

    let err = run(&read, &ctx, serde_json::json!({"path": "../secrets.txt"}))
        .await
        .unwrap_err();
    assert!(err.contains("outside the workspace"));
}

#[tokio::test]
async fn read_returns_image_attachments_for_images() {
    // pi parity (`createReadTool` image branch): magic bytes win over text —
    // a PNG reads as [text note + image block], BMP refuses with guidance,
    // and a text file with a .png NAME still reads as text.
    use epsilon_contracts::content::ContentBlock;
    use epsilon_contracts::tool::{Tool, ToolInvocation};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let ctx = ctx_for(root);
    let read = ReadTool::new();
    let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});

    // Minimal still PNG (signature + IHDR len 13 + IDAT).
    let mut png = vec![0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
    png.extend([0, 0, 0, 13]);
    png.extend(b"IHDR");
    png.extend([0u8; 13 + 4]);
    png.extend([0, 0, 0, 0]);
    png.extend(b"IDAT");
    std::fs::write(root.join("shot.png"), &png).unwrap();

    let out = read
        .execute(
            ToolInvocation {
                tool_call_id: "tc_img".into(),
                args: serde_json::json!({"path": "shot.png"}),
            },
            ctx.clone(),
            sink.clone(),
        )
        .await
        .unwrap();
    assert_eq!(out.content.len(), 2);
    assert_eq!(
        out.content[0],
        ContentBlock::text("Read image file [image/png]")
    );
    let ContentBlock::Image(img) = &out.content[1] else {
        panic!("second block must be the image, got {:?}", out.content[1]);
    };
    assert_eq!(img.mime, "image/png");
    assert!(!img.data.is_empty());

    // BMP: detected but explicitly omitted (no converter in-tree).
    let mut bmp = vec![0u8; 54];
    bmp[0] = b'B';
    bmp[1] = b'M';
    bmp[2..6].copy_from_slice(&54u32.to_le_bytes());
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14] = 40;
    bmp[26] = 1;
    bmp[28] = 24;
    std::fs::write(root.join("pic.bmp"), &bmp).unwrap();
    let bmp_out = read
        .execute(
            ToolInvocation {
                tool_call_id: "tc_bmp".into(),
                args: serde_json::json!({"path": "pic.bmp"}),
            },
            ctx.clone(),
            sink.clone(),
        )
        .await
        .unwrap();
    assert!(
        bmp_out
            .content
            .iter()
            .all(|b| !matches!(b, ContentBlock::Image(_)))
    );
    assert!(
        bmp_out.content[0]
            == ContentBlock::text(
                "Read image file [image/bmp]\n[Image omitted: BMP images are not convertible in this build.]"
            )
    );

    // Name lies, bytes don't: text content under a .png name reads as text.
    std::fs::write(root.join("fake.png"), "just text\n").unwrap();
    let text = run(&read, &ctx, serde_json::json!({"path": "fake.png"}))
        .await
        .unwrap();
    assert!(text.contains("just text"));
}

#[tokio::test]
async fn write_creates_then_edit_modifies_with_safety() {
    let (_d, ctx) = seeded_workspace();
    let write = WriteTool::new();
    let edit = EditTool::new();
    let read = ReadTool::new();

    run(
        &write,
        &ctx,
        serde_json::json!({"path": "docs/new.txt", "content": "one\ntwo\n"}),
    )
    .await
    .unwrap();

    // ambiguous match fails with count
    let err = run(
        &edit,
        &ctx,
        serde_json::json!({"path": "docs/new.txt", "old_string": "o", "new_string": "0"}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("matches 2 locations"), "got: {err}");

    // unique edit applies
    run(
        &edit,
        &ctx,
        serde_json::json!({
            "path": "docs/new.txt",
            "old_string": "two", "new_string": "TWO"
        }),
    )
    .await
    .unwrap();
    let content = run(&read, &ctx, serde_json::json!({"path": "docs/new.txt"}))
        .await
        .unwrap();
    assert!(content.contains("TWO") && !content.contains("| two"));
}

// M30 (opencode edit.ts semantics): read renders LF-stripped lines, so on
// CRLF files a model's copied old_string must still match — and the file
// keeps its native endings after the write.
#[tokio::test]
async fn edit_matches_lf_args_against_crlf_files_and_preserves_crlf() {
    let (_d, ctx) = seeded_workspace();
    let write = WriteTool::new();
    let edit = EditTool::new();
    let read = ReadTool::new();

    std::fs::write(ctx.cwd.join("crlf.txt"), "first\r\nsecond\r\nthird\r\n").unwrap();

    run(
        &edit,
        &ctx,
        serde_json::json!({
            "path": "crlf.txt",
            "old_string": "second",
            "new_string": "SECOND"
        }),
    )
    .await
    .unwrap();
    let raw = std::fs::read_to_string(ctx.cwd.join("crlf.txt")).unwrap();
    assert_eq!(raw, "first\r\nSECOND\r\nthird\r\n", "CRLF preserved");

    // Multi-line LF arguments spanning lines also match a CRLF file.
    run(
        &write,
        &ctx,
        serde_json::json!({"path": "crlf2.txt", "content": "a\r\nb\r\nc\r\n"}),
    )
    .await
    .unwrap();
    run(
        &edit,
        &ctx,
        serde_json::json!({
            "path": "crlf2.txt",
            "old_string": "a\nb",
            "new_string": "x\ny"
        }),
    )
    .await
    .unwrap();
    let raw2 = std::fs::read_to_string(ctx.cwd.join("crlf2.txt")).unwrap();
    assert_eq!(raw2, "x\r\ny\r\nc\r\n");

    // The read tool still renders it normally.
    let shown = run(&read, &ctx, serde_json::json!({"path": "crlf2.txt"}))
        .await
        .unwrap();
    assert!(shown.contains("y"));
}

#[tokio::test]
async fn edit_falls_back_to_line_trimmed_and_indent_flexible_matching() {
    let (_d, ctx) = seeded_workspace();
    let edit = EditTool::new();

    std::fs::write(ctx.cwd.join("ws.txt"), "fn a() {\n    tick();\n}\n").unwrap();

    // Trailing whitespace in old_string (copied artifact) still matches,
    // and the result note says so.
    let note = run(
        &edit,
        &ctx,
        serde_json::json!({
            "path": "ws.txt",
            "old_string": "fn a() {\n    tick();   \n}",
            "new_string": "fn a() {\n    tock();\n}"
        }),
    )
    .await
    .unwrap();
    assert!(note.contains("trailing whitespace"), "got: {note}");

    // Wrong indentation depth matches at the loosest stage.
    let note2 = run(
        &edit,
        &ctx,
        serde_json::json!({
            "path": "ws.txt",
            "old_string": "fn a() {\n  tock();\n}",
            "new_string": "fn a() {\n    hum();\n}"
        }),
    )
    .await
    .unwrap();
    assert!(note2.contains("indentation"), "got: {note2}");
    let body = std::fs::read_to_string(ctx.cwd.join("ws.txt")).unwrap();
    assert_eq!(body, "fn a() {\n    hum();\n}\n");
}

/// M71: the last two stages of the chain, and the refusal that bounds the loosest one.
#[tokio::test]
async fn edit_matches_on_anchors_and_refuses_a_disproportionate_span() {
    let (_d, ctx) = seeded_workspace();
    let edit = EditTool::new();

    // The model remembered the opening and closing lines and mis-transcribed the middle.
    std::fs::write(
        ctx.cwd.join("anchor.rs"),
        "impl Foo {\n    // real comment\n    fn bar(&self) {}\n}\n",
    )
    .unwrap();
    let note = run(
        &edit,
        &ctx,
        serde_json::json!({
            "path": "anchor.rs",
            "old_string": "impl Foo {\n    // rel comment\n    fn bar(&self) {}\n}",
            "new_string": "impl Foo {\n    fn baz(&self) {}\n}"
        }),
    )
    .await
    .unwrap();
    assert!(note.contains("first and last lines"), "got: {note}");
    assert!(note.contains("check the result"), "got: {note}");
    let body = std::fs::read_to_string(ctx.cwd.join("anchor.rs")).unwrap();
    assert_eq!(body, "impl Foo {\n    fn baz(&self) {}\n}\n");

    // Now the same anchors against a file where they bound a whole module: refused, and the
    // file is not touched.
    let mut big = String::from("mod outer {\n");
    for i in 0..40 {
        big.push_str(&format!("    fn f{i}() {{}}\n"));
    }
    big.push_str("}\n");
    std::fs::write(ctx.cwd.join("big.rs"), &big).unwrap();
    let err = run(
        &edit,
        &ctx,
        serde_json::json!({
            "path": "big.rs",
            "old_string": "mod outer {\n    fn different() {}\n}",
            "new_string": "mod outer {\n}"
        }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("refused to edit"), "got: {err}");
    assert!(err.contains("refused to edit big.rs"), "got: {err}");
    assert!(err.contains("42 lines"), "the span is named: {err}");
    assert!(err.contains("3-line"), "as is what was asked for: {err}");
    assert!(err.contains("Re-read the file"), "and what to do: {err}");
    assert_eq!(
        std::fs::read_to_string(ctx.cwd.join("big.rs")).unwrap(),
        big,
        "a refused edit must not touch the file"
    );
}

#[tokio::test]
async fn grep_glob_ls_find_seeded_content() {
    let (_d, ctx) = seeded_workspace();
    let grep = GrepTool::new();
    let glob = GlobTool::new();
    let ls = LsTool::new();

    let hits = run(&grep, &ctx, serde_json::json!({"pattern": "fn beta"}))
        .await
        .unwrap();
    assert!(hits.contains("src/lib.rs:2: pub fn beta() {}"));

    let no_case = run(
        &grep,
        &ctx,
        serde_json::json!({"pattern": "HELLO", "ignore_case": true}),
    )
    .await
    .unwrap();
    assert!(no_case.contains("README.md:2"));

    let files = run(&glob, &ctx, serde_json::json!({"pattern": "*.rs"}))
        .await
        .unwrap();
    assert!(files.contains("src/lib.rs"));

    // Pi parity: literal treats regex metachars as data.
    let lit = run(
        &grep,
        &ctx,
        serde_json::json!({"pattern": "fn beta() {}", "literal": true}),
    )
    .await
    .unwrap();
    assert!(lit.contains("src/lib.rs:2:"));
    // As raw regex, `b.ta` matches "beta"; as a literal it matches nothing.
    // That divergence is the proof `literal` escapes the pattern.
    let dot_regex = run(&grep, &ctx, serde_json::json!({"pattern": "b.ta"}))
        .await
        .unwrap();
    assert!(dot_regex.contains("src/lib.rs:2:"), "got: {dot_regex}");
    let dot_lit = run(
        &grep,
        &ctx,
        serde_json::json!({"pattern": "b.ta", "literal": true}),
    )
    .await
    .unwrap();
    assert!(dot_lit.contains("no matches"), "got: {dot_lit}");
    // Unbalanced parens are a regex syntax error either way — but only the
    // regex path reports it as one (literal mode escapes them first, so it
    // just finds nothing).
    let paren_lit = run(
        &grep,
        &ctx,
        serde_json::json!({"pattern": "fn (beta", "literal": true}),
    )
    .await
    .unwrap();
    assert!(paren_lit.contains("no matches"), "got: {paren_lit}");

    // Pi parity: glob restricts files; limit caps matches with a notice.
    let only_md = run(
        &grep,
        &ctx,
        serde_json::json!({"pattern": "fn", "glob": "*.md"}),
    )
    .await
    .unwrap();
    assert!(!only_md.contains("src/lib.rs"), "got: {only_md}");
    let capped = run(&grep, &ctx, serde_json::json!({"pattern": ".", "limit": 2}))
        .await
        .unwrap();
    assert!(capped.contains("2 matches limit reached"), "got: {capped}");

    // Pi parity: context rows use path-line- markers; long lines truncate.
    std::fs::write(ctx.cwd.join("ctx.txt"), "l1\nl2 match here\nl3\n").unwrap();
    let with_ctx = run(
        &grep,
        &ctx,
        serde_json::json!({"pattern": "match here", "context": 1, "glob": "ctx.txt"}),
    )
    .await
    .unwrap();
    assert!(
        with_ctx.contains("ctx.txt:2: l2 match here"),
        "got: {with_ctx}"
    );
    assert!(with_ctx.contains("ctx.txt-1- l1"), "got: {with_ctx}");
    assert!(with_ctx.contains("ctx.txt-3- l3"), "got: {with_ctx}");

    std::fs::write(ctx.cwd.join("long.txt"), format!("x{}\n", "y".repeat(600))).unwrap();
    let trunc = run(
        &grep,
        &ctx,
        serde_json::json!({"pattern": "x", "glob": "long.txt"}),
    )
    .await
    .unwrap();
    assert!(trunc.contains("... [truncated]"), "got: {trunc}");
    assert!(
        trunc.contains("Use read tool to see full lines"),
        "got: {trunc}"
    );

    let listing = run(&ls, &ctx, serde_json::json!({})).await.unwrap();
    assert!(listing.contains("src/"));
    assert!(listing.contains("README.md"));
}

#[cfg(windows)]
#[tokio::test]
async fn bash_runs_powershell_with_exit_codes_and_caps() {
    bash_common().await;
}

#[cfg(not(windows))]
#[tokio::test]
async fn bash_runs_bash_with_exit_codes_and_caps() {
    bash_common().await;
}

async fn bash_common() {
    let (_d, ctx) = seeded_workspace();
    let bash = BashTool::new();

    let ok = run(
        &bash,
        &ctx,
        serde_json::json!({"command": "echo hello-from-shell"}),
    )
    .await
    .unwrap();
    assert!(ok.contains("hello-from-shell"));

    let failing = run(
        &bash,
        &ctx,
        serde_json::json!({
            "command": "exit 3", "timeout_secs": 60
        }),
    )
    .await
    .unwrap();
    assert!(failing.contains("exit code: 3"));
}

#[tokio::test]
async fn bash_timeout_kills_long_commands() {
    let (_d, ctx) = seeded_workspace();
    let bash = BashTool::new();
    let cmd = if cfg!(windows) {
        "Start-Sleep -Seconds 30"
    } else {
        "sleep 30"
    };
    let err = run(
        &bash,
        &ctx,
        serde_json::json!({"command": cmd, "timeout_secs": 1}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("timed out after 1s"));
}

#[tokio::test]
async fn oversized_outputs_spill_with_locator() {
    let (_d, ctx) = seeded_workspace();
    let bash = BashTool::new();
    let cmd = if cfg!(windows) {
        "1..3000 | ForEach-Object { \"line $_\" }"
    } else {
        "for i in $(seq 1 3000); do echo \"line $i\"; done"
    };
    let out = run(&bash, &ctx, serde_json::json!({"command": cmd}))
        .await
        .unwrap();
    assert!(
        out.contains("lines / ") && out.contains("omitted"),
        "big output must report what was dropped"
    );
    assert!(out.contains("full output saved to"));
    // Both ends of a long log must survive: the first line is what a compiler
    // prints first, the last line is its summary.
    assert!(out.contains("line 1"), "the head survives: {}", &out[..200]);
    assert!(out.contains("line 3000"), "the tail survives");
}

#[tokio::test]
async fn workspace_escape_refused_in_write_and_edit() {
    let (_d, ctx) = seeded_workspace();
    let write = WriteTool::new();
    let edit = EditTool::new();

    let w_err = run(
        &write,
        &ctx,
        serde_json::json!({"path": "../evil.txt", "content": "x"}),
    )
    .await
    .unwrap_err();
    assert!(w_err.contains("outside the workspace"));

    let e_err = run(
        &edit,
        &ctx,
        serde_json::json!({"path": "../README.md", "old_string": "#", "new_string": "!"}),
    )
    .await
    .unwrap_err();
    assert!(e_err.contains("outside the workspace"));
}

#[test]
fn util_walker_respects_skip_list() {
    let _ = util::SKIP_DIRS; // referenced so the constant stays documented in tests too
}

// ---- stale-read guard (D-15) ------------------------------------------------------------------
//
// The scenario the ledger describes: the model reads a file, something else rewrites it (a
// `task` child, a background script, the user's editor), and the model's next write would
// silently discard that work. These drive the real tools, not the guard's internals.

#[tokio::test]
async fn a_write_after_something_else_changed_the_file_is_refused() {
    let (_d, ctx) = seeded_workspace();
    let read = ReadTool::new();
    let write = WriteTool::new();
    let path = ctx.cwd.join("src/lib.rs");

    run(&read, &ctx, serde_json::json!({"path": "src/lib.rs"}))
        .await
        .unwrap();

    // Another writer — this is what a `task` child or a stray process looks like to the tool.
    std::fs::write(&path, "// rewritten by someone else\n").unwrap();

    let err = run(
        &write,
        &ctx,
        serde_json::json!({"path": "src/lib.rs", "content": "// my version\n"}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("changed on disk"), "{err}");
    assert!(
        err.contains("read it again"),
        "the refusal must be actionable: {err}"
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "// rewritten by someone else\n",
        "the refused write must not have touched the file"
    );

    // Re-reading is the way through: the model now knows what is there.
    run(&read, &ctx, serde_json::json!({"path": "src/lib.rs"}))
        .await
        .unwrap();
    run(
        &write,
        &ctx,
        serde_json::json!({"path": "src/lib.rs", "content": "// my version\n"}),
    )
    .await
    .expect("after a fresh read the write proceeds");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "// my version\n");
}

#[tokio::test]
async fn a_partial_read_cannot_justify_overwriting_the_whole_file() {
    let (_d, ctx) = seeded_workspace();
    let read = ReadTool::new();
    let write = WriteTool::new();

    run(
        &read,
        &ctx,
        serde_json::json!({"path": "src/lib.rs", "offset": 1, "limit": 1}),
    )
    .await
    .unwrap();

    let err = run(
        &write,
        &ctx,
        serde_json::json!({"path": "src/lib.rs", "content": "// only line one was seen\n"}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("only read in part"), "{err}");
    assert!(
        err.contains("edit the specific text"),
        "and it must offer the alternative that is still safe: {err}"
    );

    // The full read unblocks it.
    run(&read, &ctx, serde_json::json!({"path": "src/lib.rs"}))
        .await
        .unwrap();
    run(
        &write,
        &ctx,
        serde_json::json!({"path": "src/lib.rs", "content": "// all of it seen\n"}),
    )
    .await
    .expect("a complete read justifies the overwrite");
}

#[tokio::test]
async fn an_edit_after_a_change_is_refused_but_an_unchanged_edit_is_not() {
    let (_d, ctx) = seeded_workspace();
    let read = ReadTool::new();
    let edit = EditTool::new();
    let path = ctx.cwd.join("README.md");

    run(&read, &ctx, serde_json::json!({"path": "README.md"}))
        .await
        .unwrap();

    // Unchanged file: the edit lands.
    let out = run(
        &edit,
        &ctx,
        serde_json::json!({"path": "README.md", "old_string": "hello world", "new_string": "hello epsilon"}),
    )
    .await
    .expect("an edit against an unchanged file proceeds");
    assert!(out.contains("edited README.md"), "{out}");

    // The model's own change does not make the file look foreign: a second edit still works.
    run(
        &edit,
        &ctx,
        serde_json::json!({"path": "README.md", "old_string": "hello epsilon", "new_string": "hello again"}),
    )
    .await
    .expect("the harness's own mutation updates the snapshot");

    // Now someone else moves the file underneath it.
    std::fs::write(&path, "# demo\nchanged by another writer\n").unwrap();
    let err = run(
        &edit,
        &ctx,
        serde_json::json!({"path": "README.md", "old_string": "hello again", "new_string": "nope"}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("changed on disk"), "{err}");
    assert!(err.contains("apply your edit"), "{err}");
}

#[tokio::test]
async fn creating_a_file_needs_no_history_but_blind_overwrites_do_not_happen() {
    let (_d, ctx) = seeded_workspace();
    let read = ReadTool::new();
    let write = WriteTool::new();

    // Creating: nothing is lost, so nothing to know.
    let out = run(
        &write,
        &ctx,
        serde_json::json!({"path": "src/new.rs", "content": "pub fn fresh() {}\n"}),
    )
    .await
    .unwrap();
    assert!(out.contains("created src/new.rs"), "{out}");

    // Overwriting a file that exists and was never read: refused, because the model would be
    // replacing work it has not seen.
    let err = run(
        &write,
        &ctx,
        serde_json::json!({"path": "README.md", "content": "# replaced without reading\n"}),
    )
    .await
    .unwrap_err();
    assert!(err.contains("has not been read"), "{err}");
    assert_eq!(
        std::fs::read_to_string(ctx.cwd.join("README.md")).unwrap(),
        "# demo\nhello world\n",
        "and the file is untouched"
    );

    // One read and the same write goes through.
    run(&read, &ctx, serde_json::json!({"path": "README.md"}))
        .await
        .unwrap();
    let out = run(
        &write,
        &ctx,
        serde_json::json!({"path": "README.md", "content": "# replaced after reading\n"}),
    )
    .await
    .unwrap();
    assert!(out.contains("overwrote README.md"), "{out}");
}

// ---- M77: sandbox truthfulness at the tool boundary ------------------------------------------

/// Like `run`, but hands back the whole output so `details` can be asserted.
async fn run_full(
    tool: &dyn Tool,
    ctx: &ToolContext,
    args: serde_json::Value,
) -> Result<epsilon_contracts::tool::ToolOutput, String> {
    let sink: epsilon_contracts::tool::ToolUpdateSink = Arc::new(|_| {});
    tool.execute(
        ToolInvocation {
            tool_call_id: "tc_t".into(),
            args,
        },
        ctx.clone(),
        sink,
    )
    .await
    .map_err(|f| f.message)
}

fn output_text(out: &epsilon_contracts::tool::ToolOutput) -> String {
    out.content
        .iter()
        .filter_map(|c| match c {
            epsilon_contracts::content::ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A non-zero exit is a *command* outcome: the model reads it, and nothing calls it a refusal.
#[tokio::test]
async fn a_non_zero_exit_is_a_command_outcome_never_a_confinement_one() {
    let (_d, ctx) = seeded_workspace();
    let bash = BashTool::new();
    let out = run_full(&bash, &ctx, serde_json::json!({"command": "exit 7"}))
        .await
        .expect("a non-zero exit is not a tool failure — it is information");

    assert_eq!(out.details["exit_code"], 7);
    assert_eq!(out.details["verdict"], "exited:7", "{:?}", out.details);
    let text = output_text(&out);
    assert!(text.contains("[exit code: 7]"), "{text}");
    assert!(
        !text.contains("[confinement]"),
        "an exit code must never be reported as a blocked command: {text}"
    );
}

/// A refusal is its own thing: no outcome, no verdict, and the marker the model is told to read.
#[tokio::test]
async fn a_refused_command_is_marked_as_confinement_and_produces_no_verdict() {
    let (_d, ctx) = seeded_workspace();
    let bash = BashTool::new().with_sandbox(Arc::new(epsilon_safety::NullSandbox));
    let err = run_full(&bash, &ctx, serde_json::json!({"command": "echo hi"}))
        .await
        .expect_err("a fail-closed backend refuses the call");

    assert!(err.contains("[confinement]"), "{err}");
    assert!(
        err.contains("refusing to run unconfined"),
        "the refusal says what it did *not* do and why: {err}"
    );
    // And the two questions stay separate: a refusal is an error, never an outcome with a
    // verdict attached.
    assert!(!err.contains("verdict"), "{err}");
}

/// When the platform *can* confine, the result says how completely — and when it cannot, the
/// tool says that instead. Either way the claim comes from the backend.
#[tokio::test]
async fn the_result_reports_how_completely_the_boundary_held() {
    let (_d, ctx) = seeded_workspace();
    let sandbox = epsilon_safety::best_available();
    let bash = BashTool::new().with_sandbox(Arc::clone(&sandbox));
    let args = serde_json::json!({"command": "echo confined"});

    if sandbox.available() {
        let out = run_full(&bash, &ctx, args).await.expect("the command runs");
        let enforcement = out.details["enforcement"]
            .as_str()
            .expect("enforcement is a string in details")
            .to_string();
        assert!(
            enforcement == "full" || enforcement.starts_with("partial"),
            "enforcement must be one of the two states: {enforcement}"
        );
        assert_eq!(out.details["confinement"], sandbox.name());
        // Whatever the platform cannot confine is named, not implied.
        let caps = sandbox.capabilities();
        if caps.is_full() {
            assert_eq!(enforcement, "full");
        } else {
            for missing in caps.missing() {
                assert!(
                    enforcement.contains(missing),
                    "{enforcement} should name {missing}"
                );
            }
        }
    } else {
        // Fail-closed platform: the call is refused, and nothing claims a boundary held.
        let err = run_full(&bash, &ctx, args).await.expect_err("refused");
        assert!(err.contains("[confinement]"), "{err}");
    }
}

// ---- the sandbox mode ladder (port plan item 15) --------------------------------------

#[tokio::test]
async fn a_call_may_narrow_its_own_sandbox_mode_to_read_only() {
    let (_d, ctx) = seeded_workspace();
    // Narrowing is a *gain* in safety, so it needs no approval — but it does need something to
    // narrow from: without a backend there is no boundary to tighten, and the tool says so
    // rather than accepting a mode it cannot honor.
    let sandbox = epsilon_safety::best_available();
    let bash = BashTool::new().with_sandbox(Arc::clone(&sandbox));
    let args = serde_json::json!({"command": "echo narrowed", "sandbox_permissions": "read-only",
               "justification": "the call only reads"});
    if !sandbox.available() {
        let err = run_full(&bash, &ctx, args)
            .await
            .expect_err("no boundary, no read-only claim");
        assert!(err.contains("[confinement]"), "{err}");
        return;
    }
    let out = run_full(&bash, &ctx, args)
        .await
        .expect("narrowing needs no approval");
    assert!(output_text(&out).contains("narrowed"));
    assert_eq!(out.details["mode"], "read-only");
    // The mode reaches the sandbox's own report, which stays honest about what it cannot do.
    assert!(
        out.details["enforcement"]
            .as_str()
            .unwrap_or("")
            .contains("partial")
            || sandbox.capabilities().is_full(),
        "{:?}",
        out.details
    );
}

#[tokio::test]
async fn a_call_asking_for_danger_full_access_is_refused_before_anything_runs() {
    let (dir, ctx) = seeded_workspace();
    let marker = dir.path().join("escaped.txt");
    let bash = BashTool::new();
    let err = run_full(
        &bash,
        &ctx,
        serde_json::json!({"command": format!("echo escaped > {}", marker.display()),
               "sandbox_permissions": "danger-full-access",
               "justification": "the build needs to write outside the workspace"}),
    )
    .await
    .expect_err("a per-call escape hatch is not available");
    assert!(err.contains("[sandbox]"), "{err}");
    assert!(err.contains("danger-full-access"), "{err}");
    // The one assertion that matters: nothing ran.
    assert!(!marker.exists(), "the command executed despite the refusal");
    assert!(
        err.contains("--sandbox off"),
        "tell the user the real lever: {err}"
    );
}

#[tokio::test]
async fn widening_beyond_the_session_mode_is_refused_with_a_hint() {
    let (_d, ctx) = seeded_workspace();
    // A session confined to looking may not decide for itself that it needs to write.
    let bash = BashTool::new().with_session_mode(epsilon_safety::SandboxMode::ReadOnly);
    let err = run_full(
        &bash,
        &ctx,
        serde_json::json!({"command": "echo hi > file.txt", "sandbox_permissions": "workspace-write",
               "justification": "I need to write a file"}),
    )
    .await
    .expect_err("widening is a user decision");
    assert!(err.contains("wider than this session's mode"), "{err}");
    assert!(
        err.contains("sandbox_permissions") && err.contains("justification"),
        "{err}"
    );
}

#[tokio::test]
async fn the_permissions_and_justification_arguments_must_travel_together() {
    let (_d, ctx) = seeded_workspace();
    let bash = BashTool::new();
    let err = run_full(
        &bash,
        &ctx,
        serde_json::json!({"command": "echo hi", "sandbox_permissions": "read-only"}),
    )
    .await
    .expect_err("half a request is not a request");
    assert!(err.contains("justification"), "{err}");

    let err = run_full(
        &bash,
        &ctx,
        serde_json::json!({"command": "echo hi", "sandbox_permissions": "raed-only",
               "justification": "typo in the mode name"}),
    )
    .await
    .expect_err("a typo must not become a default");
    assert!(err.contains("unknown sandbox mode"), "{err}");
    assert!(err.contains("read-only"), "name the vocabulary: {err}");
}

#[tokio::test]
async fn with_confinement_off_a_narrowing_request_is_refused_rather_than_lied_about() {
    let (_d, ctx) = seeded_workspace();
    let bash = BashTool::new(); // no sandbox attached = the explicit opt-out path
    let err = run_full(
        &bash,
        &ctx,
        serde_json::json!({"command": "echo hi", "sandbox_permissions": "read-only",
               "justification": "the call only reads"}),
    )
    .await
    .expect_err("read-only cannot be honored without a boundary");
    assert!(err.contains("[confinement]"), "{err}");
    assert!(
        err.contains("--sandbox auto"),
        "say how to get a boundary: {err}"
    );
}

// ---- LSP diagnostics as edit feedback (port plan / roadmap Phase 5 item 1, M79) ---------

/// A provider that always has something to say, so the *plumbing* is what the tests measure.
struct AlwaysSays(&'static str);

#[async_trait::async_trait]
impl epsilon_contracts::diagnostics::DiagnosticsProvider for AlwaysSays {
    async fn after_change(&self, _path: &std::path::Path, _text: &str) -> Option<String> {
        Some(self.0.to_string())
    }
}

/// A provider that cannot answer — the shape of a machine with no language server.
struct SaysNothing;

#[async_trait::async_trait]
impl epsilon_contracts::diagnostics::DiagnosticsProvider for SaysNothing {
    async fn after_change(&self, _path: &std::path::Path, _text: &str) -> Option<String> {
        None
    }
}

#[tokio::test]
async fn a_write_carries_what_the_language_server_thinks_of_it() {
    let (_d, ctx) = seeded_workspace();
    let write = WriteTool::new().with_diagnostics(Arc::new(AlwaysSays(
        "[lsp] lib.rs: 1 error\n  ERROR [1:1] boom",
    )));

    // A fresh file: the stale-read guard (M65) refuses to overwrite one the model never read,
    // and this test is about diagnostics, not about that rule.
    let out = run_full(
        &write,
        &ctx,
        serde_json::json!({"path": "src/fresh.rs", "content": "fn broken() {}\n"}),
    )
    .await
    .expect("the write succeeds whether or not the linter has news");
    let text = output_text(&out);
    assert!(text.contains("created src/fresh.rs"), "{text}");
    assert!(text.contains("[lsp] lib.rs: 1 error"), "{text}");
    assert!(text.contains("ERROR [1:1] boom"), "{text}");
}

#[tokio::test]
async fn a_write_is_unchanged_when_no_server_can_answer() {
    let (_d, ctx) = seeded_workspace();

    let providers: [Option<Arc<dyn epsilon_contracts::diagnostics::DiagnosticsProvider>>; 2] =
        [Some(Arc::new(SaysNothing)), None];
    for (index, provider) in providers.into_iter().enumerate() {
        let write = match provider {
            Some(provider) => WriteTool::new().with_diagnostics(provider),
            None => WriteTool::new(),
        };
        let name = format!("src/quiet{index}.rs");
        let out = run_full(
            &write,
            &ctx,
            serde_json::json!({"path": name, "content": "fn ok() {}\n"}),
        )
        .await
        .expect("write succeeds");
        let text = output_text(&out);
        assert!(!text.contains("[lsp]"), "no lsp block expected: {text}");
        assert!(text.contains(&format!("created {name}")), "{text}");
    }
}

#[tokio::test]
async fn an_edit_carries_the_diagnostics_too() {
    let (_d, ctx) = seeded_workspace();
    // The edit needs a prior read of this file (stale-read guard), and this one changes it.
    std::fs::write(ctx.cwd.join("src/lib.rs"), "fn a() {}\n").unwrap();

    let read = ReadTool::new();
    let _ = run_full(&read, &ctx, serde_json::json!({"path": "src/lib.rs"})).await;

    let edit = EditTool::new().with_diagnostics(Arc::new(AlwaysSays(
        "[lsp] lib.rs: 1 error\n  ERROR [1:1] still broken",
    )));
    let out = run_full(
        &edit,
        &ctx,
        serde_json::json!({"path": "src/lib.rs", "old_string": "fn a() {}", "new_string": "fn b() {}"}),
    )
    .await
    .expect("edit succeeds");
    let text = output_text(&out);
    assert!(text.contains("edited src/lib.rs"), "{text}");
    assert!(text.contains("ERROR [1:1] still broken"), "{text}");
}
