# M199 — negative controls for workspace containment

Both controls were run in place against the candidate, then restored with a
`sha256sum -c` byte-identity check. A control that has never been seen red proves
nothing, and the pre-fix RED for the `references.rs` half was available because that
bypass exists at HEAD as well as in the slice.

## Control 1 — the sensitive-path bypass (`references.rs`)

Reverting one line, the `real` half of the check:

```rust
-    if is_sensitive(&resolution.path) || resolution.real.as_deref().is_some_and(is_sensitive) {
+    if is_sensitive(&resolution.path) {
```

`cargo test -p epsilon-core --lib references::tests::a_link_cannot` — **FAILED**, for
the reason the test names:

```
thread '…a_link_cannot_carry_an_innocent_name_into_a_sensitive_file' panicked at
  crates/epsilon-core/src/references.rs:519:17:
@vcs/ reached .git through a link: "config"
```

The body is `config` — the attachment was **not** refused, so `.git/config` was on
its way into the prompt behind a link named `vcs`.

## Control 2 — containment itself (`paths.rs`)

`resolve_for_access` was made to answer lexically, which is what HEAD's
`resolve_in_workspace` did. Two named tests went red, and the tools-side one is the
one that matters:

```
paths::tests::a_symlink_pointing_outside_is_reported_with_its_target
  a symlink out of the workspace must be outside:
  Resolution { path: "…\root\link\id_rsa", inside: true, real: None }

util::tests::a_symlink_out_of_the_workspace_is_refused_and_named
  a link out must be refused: "…\root\creds\id_rsa"
```

`expect_err` **did not get an error**: the lexical build allowed the path. That is
the whole defect — `write` is `Allow` by default, so nothing downstream would have
asked.

## What this box could and could not run

`--nocapture` on the shipped test reports which link shapes actually executed:

```
skipped: this platform would not create a file link
```

- **Directory junction → `.git`**: RAN. A junction needs no privilege, so the
  privilege-free shape is the one carrying the weight of the portable assertions.
- **File symlink → `.env`**: SKIPPED, loudly. Windows refuses an unprivileged file
  symlink, and `test_support::file_link` says so rather than passing vacuously.
