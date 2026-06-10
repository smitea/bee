-- Multi-stream analytics: 3 test-fixture streams + JOIN + GROUP BY.
--
-- This is the 3rd of 3 S41 demos. Exercises:
-- - 3 test-fixture streams (clicks, views, purchases) synthesized by
--   `generate_events(schema, count, seed)`. The preprocessor
--   expands each `FROM generate_events(...)` to a literal
--   `VALUES (uid, ts), (uid, ts), ...` table at preprocessor
--   time (the same LCG the UDF would run, but emitted as a
--   native `VALUES` table to avoid DataFusion 50's UNNEST-of-
--   List-of-Struct quirks; see
--   `crates/bee-dsl-sql/src/preprocess.rs` →
--   `expand_generate_events_in_from` for the history).
-- - INNER JOIN across the 3 streams on user_id. The temporal
--   ASOF semantic is exercised in the asof.rs unit tests
--   (the end-to-end ASOF test in
--   `crates/bee-dsl-sql/src/asof.rs` is `#[ignore]`d because
--   DataFusion 50's physical plan does not implement
--   OuterReferenceColumn for correlated subqueries; see
--   issue #318). The translator is correct (parses + plans;
--   only the physical execution blocks). The demo's JOIN
--   form here is the non-LATERAL form so the demo runs
--   end-to-end on DataFusion 50.
-- - GROUP BY aggregation over the joined stream.
-- - EMIT INTO console (Task 9a) for output.
--
-- Stream counts:
--   clicks:    1000 events
--   views:      500 events
--   purchases:  250 events
--
-- Determinism: each stream takes a different seed (42/43/44) so the
-- preprocessor's LCG produces distinct event sequences. The same
-- seeds across runs give the same output.

CREATE SOURCE clicks AS
SELECT user_id, ts FROM generate_events(0, 1000, 42);

CREATE SOURCE views AS
SELECT user_id, ts FROM generate_events(0, 500, 43);

CREATE SOURCE purchases AS
SELECT user_id, ts FROM generate_events(0, 250, 44);

CREATE VIEW joined AS
SELECT
    c.user_id AS c_user_id,
    c.ts      AS c_ts,
    v.user_id AS v_user_id,
    v.ts      AS v_ts,
    p.user_id AS p_user_id,
    p.ts      AS p_ts
FROM clicks c
INNER JOIN views     v ON c.user_id = v.user_id
INNER JOIN purchases p ON c.user_id = p.user_id;

CREATE VIEW aggregated AS
SELECT c_user_id, count(*) AS event_count
FROM joined
GROUP BY c_user_id;

EMIT INTO console
SELECT * FROM aggregated ORDER BY c_user_id LIMIT 10;
