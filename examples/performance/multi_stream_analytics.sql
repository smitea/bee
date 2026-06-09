-- Multi-stream analytics: 3 test-fixture streams + ASOF JOIN + GROUP BY.
--
-- This is the 3rd of 3 S41 demos. Exercises:
-- - 3 test-fixture streams (clicks, views, purchases) synthesized by
--   `generate_events` (the S41 scalar UDF that returns
--   `Struct<user_id, ts>`). The preprocessor auto-wraps the UDF call
--   in `UNNEST(... ) AS t(user_id, ts)` so the struct fields become
--   columns in the FROM clause (DataFusion 50 has no UDTF support;
--   UNNEST is the canonical way to expand a scalar UDF's struct
--   result into a table).
-- - LEFT ASOF JOIN (Task 9b) translated to LEFT JOIN LATERAL ... LIMIT 1.
--   The translator in crates/bee-dsl-sql/src/asof.rs is now correct
--   (fixes in 5c7ea37 + 5809864: paren-aware cond_end, nested-subquery
--   handling, leading-LEFT/RIGHT/INNER stripping). This demo exercises
--   the S41 ASOF extension end-to-end through the translator,
--   preprocess_sql_v2, and the console sink. Whether the executor can
--   physically run the LATERAL is a separate question tracked by the
--   `#[ignore]`d end-to-end ASOF test (LATERAL physical plan limitation).
-- - GROUP BY aggregation over the joined stream.
-- - EMIT INTO console (Task 9a) for output.
--
-- Stream counts:
--   clicks:    1000 events
--   views:      500 events
--   purchases:  250 events
-- (10x the S41 MVP baseline so the LATERAL JOIN work is non-trivial.)
--
-- Determinism: each stream takes a different seed (42/43/44) so the
-- pseudo-random LCG in `generate_events_impl` produces distinct event
-- sequences. The same seeds across runs give the same output.

CREATE SOURCE clicks AS
SELECT user_id, ts FROM generate_events(0, 1000, 42);

CREATE SOURCE views AS
SELECT user_id, ts FROM generate_events(0, 500, 43);

CREATE SOURCE purchases AS
SELECT user_id, ts FROM generate_events(0, 250, 44);

CREATE VIEW joined AS
SELECT c.user_id AS c_user_id, c.ts AS c_ts, v.user_id AS v_user_id, v.ts AS v_ts
FROM clicks c
LEFT ASOF JOIN views v ON c.user_id = v.user_id AND c.ts >= v.ts;

CREATE VIEW aggregated AS
SELECT c_user_id, count(*) AS event_count
FROM joined
GROUP BY c_user_id;

EMIT INTO console
SELECT * FROM aggregated ORDER BY c_user_id LIMIT 10;
