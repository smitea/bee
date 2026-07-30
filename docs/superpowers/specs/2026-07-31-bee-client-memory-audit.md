# Memory Audit — Bee Client (Bee Client slice 9)

**Scope**: components and stores touched by the seed-demo feature plus a
backward scan of the existing component set for missing cancel / cleanup
guards. The brief is "report only — do not modify code", so this document
captures findings and recommendations for a follow-up patch.

**Audit date**: 2026-07-31
**Audit version**: pass 1 (conservative)

## Summary

| # | Severity | File | Issue |
|---|----------|------|-------|
| 1 | Low | `app/src/components/NavTree.tsx` (new code) | `onSeedDemo` async handler does not guard against unmount; `setSeeding(false)` in `finally` may run after unmount. |
| 2 | Low | `app/src/components/DashboardEditor.tsx` | `mousemove` / `mouseup` document listeners are only removed by `onUp`; if the component unmounts mid-drag, both listeners leak. |
| 3 | Low | `app/src/state/searchStore.ts` | Module-level `debounceHandle` is never cleared on store re-creation (HMR / tests). |
| 4 | Info | `app/src/components/ContextMenu.tsx` | `useEffect` cleanups are correct. (No issue — kept for reference.) |
| 5 | Info | `app/src/components/ActivityDialog.tsx` | `window.addEventListener("keydown", onKey)` is correctly removed in the cleanup. (No issue — kept for reference.) |
| 6 | Info | `app/src/pages/PipelineDetail.tsx` | `useEscape` hook adds `window.addEventListener("keydown", onKey)` with proper cleanup. (No issue — kept for reference.) |
| 7 | Info | `app/src/components/SettingsModal.tsx` | All 5 `useEffect` blocks that schedule async work guard with `cancelled` and clear `setTimeout` in cleanup. (No issue — kept for reference.) |
| 8 | Info | `app/src/App.tsx` | `setInterval` for connection status is cleared on cleanup; `cancelled` flag guards against late `setStatus`. (No issue — kept for reference.) |
| 9 | Info | `app/src/pages/DashboardPanel.tsx` | `setInterval` is cleared; `cancelled` flag guards against late `setState`. (No issue — kept for reference.) |
| 10 | Info | `app/src/components/SearchBox.tsx` | `setTimeout` debounce is cleared in cleanup. (No issue — kept for reference.) |

**No critical leaks found.** The Bee Client component set already follows
the "guarded async + cleanup" pattern well; the gaps are localised.

## New findings (from this pass)

### 1. `NavTree.onSeedDemo` — no unmount guard

`app/src/components/NavTree.tsx:155-164`

```ts
const onSeedDemo = async () => {
  if (seeding) return;
  setSeeding(true);
  try {
    await seedDemo();
  } catch (e) {
    console.error("seed demo failed", e);
  } finally {
    setSeeding(false);
  }
};
```

**Risk**: If the user clicks "Seed demo" and then closes the sidebar
(e.g., navigates away or the workspace tab is closed) before the IPC
round-trip resolves, the `finally` block calls `setSeeding(false)` on a
component that has unmounted. React 18 silently drops this in production,
but logs a warning in dev. It also keeps the closure alive even after
unmount.

**Recommended fix** (deferred per task brief):

```ts
useEffect(() => {
  let cancelled = false;
  return () => { cancelled = true; };
}, []);

const onSeedDemo = async () => {
  if (seeding) return;
  setSeeding(true);
  try {
    await seedDemo();
    if (cancelled) return;
  } catch (e) {
    if (cancelled) return;
    console.error("seed demo failed", e);
  } finally {
    if (!cancelled) setSeeding(false);
  }
};
```

### 2. `DashboardEditor` drag listeners — no unmount cleanup

`app/src/components/DashboardEditor.tsx:373-374`

```ts
document.addEventListener("mousemove", onMove2);
document.addEventListener("mouseup", onUp);
```

**Risk**: If the user starts dragging a panel and the
`<DashboardEditor>` unmounts (e.g., the parent unmounts due to a route
change), neither `mousemove` nor `mouseup` is removed. The listeners
keep a reference to the (now-dead) `onCommit` / `onResize` closures
until the next mouseup event fires.

**Recommended fix**: track the `useEffect` lifecycle owner and register
removes in a `useEffect`; or attach a cleanup that removes both
listeners on unmount.

### 3. `searchStore` module-level `debounceHandle`

`app/src/state/searchStore.ts:31-32`

```ts
let requestSeq = 0;
let debounceHandle: ReturnType<typeof setTimeout> | null = null;
```

**Risk**: In Vite HMR, a fresh `useSearch` store replaces the previous
one, but the pending `setTimeout` still holds the *old* `get()` closure
and will resolve by calling `get().runSearchNow(...)` on the new store.
The new store cannot cancel the in-flight request — at worst a stale
search result is dropped because of `requestSeq`. Not a memory leak per
se, but a correctness / HMR papercut.

**Recommended fix**: capture the request id inside the timeout callback
and compare against `requestSeq` before calling `runSearchNow`.

## Reference: patterns that are correct

For the next maintainer, here are the patterns the codebase already uses
correctly. *(No action required.)*

- `SettingsModal.tsx` `useEffect` blocks: use `cancelled = false` toggled
  in the cleanup, and `clearTimeout` for any `setTimeout` they schedule.
- `App.tsx` connection poll: `setInterval` is cleared; `cancelled` flag
  prevents `setStatus` after unmount.
- `DashboardPanel.tsx` poll: same pattern as `App.tsx`.
- `SearchBox.tsx` debounce: `clearTimeout` on cleanup.
- `ContextMenu.tsx` outside-click handler: `document.addEventListener` in
  `useEffect`, both listeners removed in the cleanup.
- `ActivityDialog.tsx` and `PipelineDetail.tsx`: `window.addEventListener("keydown")`
  paired with `removeEventListener` in cleanup.
- `ClusterTopology.tsx`: only `useState` / `useMemo` / `setRfNodes`,
  no subscription leaks.

## Recommendations

1. Patch `NavTree.onSeedDemo` with the `cancelled` flag shown above.
2. Patch `DashboardEditor` drag start to attach listeners inside a
   `useEffect` keyed on the drag-id, so cleanup removes them on unmount.
3. Optional: scope `searchStore`'s `debounceHandle` to the store closure
   (move into `create` payload) so it is naturally garbage-collected.

No immediate user-visible break. The fixes are batchable into a single
follow-up slice labelled "memory cleanup pass".
