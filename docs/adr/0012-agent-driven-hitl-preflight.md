# 0012: Agent-driven HITL pre-flight (the S33 sign-off pattern)

When a story is marked **HITL** (Human-In-The-Loop) and the project's `docs/stories.md` says "marked done only after the seed user signs off", and the project does NOT have a real seed user in the loop right now (this is the situation for Bee's S33 quant-trading HITL milestone on 2026-06-10), the agent has two options:

1. **Skip the sign-off and leave the story "awaiting HITL review" forever.** This is the conservative option; it preserves the formal "HITL only" gate. The downside: S33 and everything downstream of it (the 1.0 narrative anchor) cannot be marked done. The project is functionally blocked.

2. **Drive the sign-off on the agent's behalf to the extent possible, and capture the production-deployment gaps as new stories.** This is the option Bee took for S33 on 2026-06-10 (commit `0e6a95b`).

We adopt **option 2 (the S33 sign-off pattern) (A)** because:

- A story's acceptance criteria split into **code-level claims** (which the agent CAN verify) and **production-deployment claims** (which require a real human with real credentials + a real cluster + a 24h wall-clock window). Conflating the two into a single "the agent did all the work, so it's done" check is dishonest; refusing to do any verification on the agent's part is wasteful.

- The agent can verify code-level claims by reading the code + running the relevant unit + integration tests + the `BEE_DRY_RUN=1` mode of the demo script. The agent CANNOT verify production-deployment claims (real data flowing, 24h stability, multi-node failover) without the real prerequisites — but the agent CAN document the **gap** as a new story (S33.1: multi-node cluster, S33.2: 24h live soak) so the seed user (or a follow-up agent) has a clear scope and acceptance criteria when they pick it up.

- The pattern keeps the HITL gate **honest**: a real human's sign-off is still required for S33 to flip from "partial" to "done". The agent's "partial sign-off" is clearly marked partial (the sign-off form has explicit "agent cannot fill — needs a real user" entries in the fields that need a human). The story is NOT marked done — it's marked "Pre-flight green; production deferred to seed user".

- The pattern produces a **good-enough baseline** for the seed user: the path bugs are found and fixed, the dry-run path is verified, the code-level ADR table is filled with code + test references. The seed user doesn't re-discover the path; they fill the remaining blanks in the form.

## How the pattern works (concretely, with the S33 case)

1. **Read the story's sign-off form** (the table with the seed user fields). Identify the rows that are "code-level" vs "production-level" by reading the row labels + the story's body for context.

2. **Run the dry-run path of the demo script.** For S33, this is `BEE_DRY_RUN=1 bash scripts/demo-quant-prod.sh` (an env-var-gated mode that exercises every step except the actual InfluxDB / MongoDB writes and the multi-node failover). Capture the 23/23 result.

3. **For each "code-level" claim, find the code + test that backs it.** Build a table: ADR number | claim | code reference | test reference | status. Mark each row as `✓ code-verified` if the reference exists and the test passes.

4. **For each "production-level" claim, mark it `N` (not observed) with a brief reason.** Be explicit about WHY the agent can't verify (no real credentials, no real cluster, no 24h window).

5. **For each gap surfaced by the above (anything `N` or any path bug fixed in the agent's run), write a new story.** The story has its own Type (AFK if the agent can drive; HITL if a real human is needed), Blocked-by, ADRs, Scope, Out-of-scope, Acceptance criteria, and Deliverables. S33's gaps became S33.1 (multi-node cluster) and S33.2 (24h live soak) — each one a self-contained scope that can be picked up independently.

6. **Update the story's status line + sign-off form** to reflect the new state. The story is NOT marked `done`; it's marked `partial` with a clear "Pre-flight: 23/23 green. Production: deferred to seed user" status. The form has explicit "agent cannot fill" entries in the human-only fields.

7. **Commit the story update, the demo-script fix (if any), and the new follow-up stories together.** The S33 case produced 2 commits: `S33 (a): fix demo-quant-prod.sh plugin path + ta-indicators rename` (the path-bug fix that the dry-run surfaced) and `S33 (HITL sign-off attempt): partial — code-verified, production deferred` (the story update).

## When to apply the pattern

The pattern applies when ALL of the following hold:

- A story is marked **HITL**.
- The project's `docs/stories.md` says the HITL gate is "the seed user signs off".
- The project does NOT currently have an active seed user in the loop.
- The story's acceptance criteria are a mix of (a) code-level claims and (b) production-deployment claims (the typical case for any "real production validation" story).

When the pattern does NOT apply:

- A story is AFK (no human needed) → just implement and verify.
- A story is HITL and a real human is in the loop → wait for the human; the agent should not pre-empt the sign-off.
- A story's only acceptance criteria are "real production validation" with no code-level surface → the agent cannot do anything; the story stays "awaiting HITL review" forever (this is the S33-only subcase where the agent CAN do pre-flight because there IS a code-level surface).

## Consequences

- **Stories that block on HITL can be partially signed off** when an active seed user is not available. The pre-flight is honest (it marks the production-only rows as `N`), and the partial sign-off is clearly labelled. Future readers do not mistake "partial sign-off" for "story done".
- **Path bugs are surfaced early.** The S33 dry-run found two pre-existing path bugs in `scripts/demo-quant-prod.sh` (`plugin_dylib` looked in the wrong directory; the build step referenced the pre-restructure `ta-indicators` name). These would have blocked the seed user on first run. The agent's run catches them BEFORE the seed user wastes time.
- **The new follow-up stories have a clear scope.** S33.1 (multi-node cluster) and S33.2 (24h live soak) each have their own Type, Blocked-by, Scope, Out-of-scope, Acceptance criteria, Deliverables. A future agent or human picking up S33.1 doesn't have to re-read S33 to figure out what to do.
- **The pattern can be re-used for any future HITL story** that has a code-level surface. S33 is the first instance; S33.1's eventual "S33.1 HITL sign-off" (when its TCP-cluster integration is ready for production review) can use the same pattern.
- **The pattern does NOT replace the real seed user.** The S33 sign-off form's "Real money signals observed" / "Failover verified (production)" / etc. rows are still marked `N` and require a real human. The agent's pre-flight is a **means** to verify, not the verification itself.
- **The pattern is documented in the story's status line** (e.g. "Pre-flight: 23/23 green. Production: deferred to seed user") so a future reader of `docs/stories.md` knows the story's state at a glance, without having to re-read the full sign-off form.

## Alternatives considered

- **B. Skip the sign-off entirely; leave S33 as "awaiting HITL review" forever.** This was the conservative option. The downside: nothing downstream of S33 can be marked done, the 1.0 narrative can't anchor on a signed-off deployment, and the demo script's "BEE_MULTINODE=1 failover demo deferred to 1.x" stays in the script's header forever. **Rejected** as too pessimistic given the S33 acceptance criteria's clear code-level surface.

- **C. Mark S33 done and ship.** This was the optimistic option. The downside: it's dishonest. The form's "Real money signals observed" / "Failover verified" rows would be marked `Y` when they were not actually observed. **Rejected** as misleading.

- **D. Wait for a real human.** This is the formally correct option, but it leaves the project blocked indefinitely. **Rejected** because the project is currently in a state where there is no active seed user and the project owner wants forward progress on the docs + demo-script work.

The chosen option A is the **middle ground**: drive the code-level pre-flight honestly, surface the production gaps as new stories, and document the partial state clearly so a real seed user can pick up the remaining work without re-discovering the path.
