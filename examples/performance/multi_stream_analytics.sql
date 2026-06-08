-- Multi-stream analytics: 3 test-fixture streams + JOIN + GROUP BY.
--
-- This is the 3rd of 3 S41 demos. Exercises:
-- - 3 inline VALUES sources (clicks, views, purchases) — these stand in
--   for the test-fixture streams the plan called for via generate_events
--   (see "ASOF JOIN fallback" below).
-- - LEFT JOIN (a regular equi-join; see "ASOF JOIN fallback" below).
-- - GROUP BY aggregation over the joined stream.
-- - EMIT INTO console (Task 9a) for output.
--
-- ASOF JOIN fallback:
-- The plan's design (Task 9b) specifies LEFT ASOF JOIN translated to
-- LEFT JOIN LATERAL ... LIMIT 1. However, on closer inspection of the
-- translator's output, the format! call in crates/bee-dsl-sql/src/asof.rs
-- (translate_asof) uses named arguments {translated_op} and {direction}
-- whose actual bindings are named translated_ineq_op and order_direction.
-- The resulting SQL has stray identifiers and unparseable fragments:
--   "LEFT LEFT JOIN LATERAL ... AND v.ts) <= c.ts ... DESC LIMIT 1) ... v ON TRUE"
-- which DataFusion rejects with ParserError("Expected: ), found: , at ...").
-- crates/bee-dsl-sql/ is outside the S41 touch list for this task, so
-- the ASOF translator bug is left in place and the demo falls back to
-- a regular LEFT JOIN equi-join.
--
-- generate_events fallback:
-- The plan called for 3 test-fixture streams via `generate_events`. That
-- UDF is registered as a scalar UDF returning a Struct<user_id, ts>.
-- DataFusion 50 cannot UNNEST a struct-typed UDF result and the
-- preprocessor's `rewrite_generate_series_in_from` only auto-wraps
-- `generate_series` (not `generate_events`) in UNNEST. Even with the
-- `test-fixtures` feature enabled, all three S41 demos (fibonacci,
-- prime_sieve, multi_stream_analytics) fail at the planner with
-- "unnest() can only be applied to array, struct and null" or
-- "Invalid function 'generate_events'". The fix is to either
-- (a) generalize the UNNEST rewrite to other UDFs in the preprocessor,
-- (b) register generate_events as a UDTF instead of a scalar UDF, or
-- (c) register generate_events as returning List<Struct<...>> and
--     document the cast. None of these are in scope for Task 12. The
--     inline VALUES tables below are the workaround: they let the
--     rest of the S41 demo chain (CREATE SOURCE / CREATE VIEW /
--     JOIN / GROUP BY / EMIT INTO console) still be exercised
--     end-to-end.

CREATE SOURCE clicks AS
SELECT user_id, ts FROM (VALUES
    (1, 100), (1, 200), (2, 300), (2, 400), (3, 500),
    (3, 600), (4, 700), (4, 800), (5, 900), (5, 1000)
) AS t(user_id, ts);

CREATE SOURCE views AS
SELECT user_id, ts FROM (VALUES
    (1, 50), (1, 150), (2, 250), (2, 350), (3, 450),
    (3, 550), (4, 650), (4, 750), (5, 850), (5, 950)
) AS t(user_id, ts);

CREATE SOURCE purchases AS
SELECT user_id, ts FROM (VALUES
    (1, 500), (2, 600), (3, 700), (4, 800), (5, 900)
) AS t(user_id, ts);

CREATE VIEW joined AS
SELECT c.user_id AS c_user_id, c.ts AS c_ts, v.user_id AS v_user_id, v.ts AS v_ts
FROM clicks c
LEFT JOIN views v ON c.user_id = v.user_id;

CREATE VIEW aggregated AS
SELECT c_user_id, count(*) AS event_count
FROM joined
GROUP BY c_user_id;

EMIT INTO console
SELECT * FROM aggregated ORDER BY c_user_id LIMIT 10;
