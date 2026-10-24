# Models: picking, live listings, retirement

`/models` opens the picker. Rows come from the bundled catalog plus, for
providers with a credential, a live listing fetched from the provider's own
`/models` endpoint. Unknown ids you configure are kept as struck-through
`[unavailable]` rows rather than dropped, so the running state stays visible
in the surface that changes it.

**A live listing is not a route.** The bundled catalog carries the wire a
request needs; an id that appears only in a provider's `/models` page is shown
struck and cannot be applied, saved as the default, or favourited. Before this
rule the picker wrote such an id into `[model] id` and the next launch failed
with `unknown model`. Use a catalogued model, or declare a custom provider whose
`<name>/` prefix routes the id.

## Live listings

Opening `/models` (or entering a provider) fetches that provider's live
models once per session in the background; `Ctrl+G` refetches on demand.
Auto-fetch skips listings younger than 60 minutes — manual refresh always
refetches. A fetched listing is cached on disk, so the next launch starts
from it (marked `from startup cache, <age> ago — auto-refreshes on open`) instead of an empty list; a failed fetch
keeps the last good listing with its real age rather than clearing it.
Offline builds never fetch and browse the bundle only.

`/catalog` reports the bundled snapshot date plus each live listing's age
(`refreshed <age> ago (this session)`, `from startup cache, <age> ago — auto-refreshes on open`, or `not fetched yet — opening refreshes`). The provider's
model list repeats the same line above its key hints, so the age of the list is
visible while choosing from it.

## Retired models

Providers retire model ids. A retired id — dropped from the bundle because
the registry marks it deprecated — badges `[retired]` instead of
`[unavailable]`: "we used to list this" versus "we never knew this". Either
way the row is struck and cannot be selected; pick the replacement explicitly,
nothing auto-substitutes. The badge tolerates bare and provider-scoped
spellings, but retirement never follows a name across providers.
