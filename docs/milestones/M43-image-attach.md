# M43 — pi-parity: image attach (`harness/tools/image.ts` + read image branch)

Eleventh port batch against the 2026-09-03 parity audit (tools findings —
image support). Epsilon's `read` refused anything with a NUL byte as
"binary": screenshots, diagrams, and photos were unreadable to the model.
The `ImageRef` content blocks already existed in contracts (and webfetch
already sniffed image MIME) — the read path just never produced them.

## What landed

1. **Detection module** (`epsilon-tools/src/image.rs`, `pub`): byte-exact
   port of pi's `detectSupportedImageMimeType` + chunk walkers — JPEG (with
   pi's `0xF7` fourth-byte exclusion), still PNG (IHDR, `acTL`-before-`IDAT`
   means animated ⇒ `None`), GIF, WebP, validated BMP. Unit tests cover all
   five + the animated-PNG and `0xF7` exclusions.
2. **Read branch** (`files.rs`): magic bytes are checked BEFORE text, so
   names never decide — a `.png` containing text reads as text, a nameless
   blob with PNG bytes reads as image. Images return `[text note + Image
   block]` with base64 payload (pi's `encodeBase64`); BMP returns the
   explicit-omission note (pi needs an injected converter for BMP too — we
   have none, so we say so instead of shipping undecodable bytes).
3. **Dependency**: `base64 = "=0.23.1"` pinned in workspace deps (already in
   Cargo.lock transitively via rmcp/reqwest — no new supply chain, just a
   direct pin per policy) + `epsilon-tools` dep.
4. **Tests**: `read_returns_image_attachments_for_images` (PNG attach + BMP
   note + lying-name reads as text).

## Key decisions

- Deliberately NOT ported: resize/convert (`image-resize.ts`,
  `image-convert.ts`, `image-process.ts`) — those need an image-codec crate
  (pi shells to an injected processor; the coding-agent default uses sharp).
  No codec is in the tree and the dependency policy demands ADR-level reason
  for a new one. Large images go inline unscaled for now; the resize seam is
  where the `ReadImageProcessor` would plug in (a `ToolHooks` after-hook or a
  read option — left for the enhancement phase with the codec choice).
- BMP detected-but-omitted (not `None` → text fallback): a BMP decoded as
  text would be mojibake; the explicit note tells the model what happened.
  Animated PNG returns `None` (pi parity) → falls through to the binary
  refusal, which is honest: we can't show frame 1 either.
- One-char fork from pi: `pixelOffset >= declaredSize` → `>` (offset ==
  size is a valid empty-pixel-data header; pi rejects it, we accept). Only
  affects header-only fixtures, never real files. Noted, not hidden.

## Gotchas

- Test fixture bug (mine, twice): a 54-zero-byte "BMP" fails pi's own
  validation (`declared_size == 0` is OK but `pixel_offset(0) < 14+40`
  fails). The fixture must carry real declared-size + pixel-offset fields —
  exactly the kind of magic-byte subtlety this module exists to get right.
- `write_file` for `image.rs` then `patch` anchor drift on `lib.rs` (CRLF):
  re-read regions before retrying (standing discipline, still applies).

## Verification

- `cargo test -p epsilon-tools`: 34 lib (2 new image tests) + 11
  integration (1 new read-image test), green.
- `cargo fmt --check` clean, workspace clippy zero warnings.
