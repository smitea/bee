-- Phase 1: emit all integers in [2, 10^4]
-- S44 demo: range reduced from 10^8 + 1229 phases to 10^4 + 25
-- phases so the demo finishes in seconds. The sieve is
-- complete (covers primes <= sqrt(10^4) = 100); n_primes should
-- equal pi(10^4) = 1,229. For the full 10^8 sieve (~3 min,
-- 5,761,455 primes), see prime_sieve_full.sql or set BEE_FULL_SIEVE=1.
CREATE SOURCE naturals AS
SELECT n FROM generate_series(2, 10000);

-- 25 sieving phases (one per prime <= 100, the largest prime <= sqrt(10^4)).
-- Each phase filters multiples of its prime; survivors at the end
-- are exactly the primes <= 10^4.

CREATE VIEW sieved_2 AS
SELECT n FROM naturals WHERE n = 2 OR n % 2 != 0;
CREATE VIEW sieved_3 AS
SELECT n FROM sieved_2 WHERE n = 3 OR n % 3 != 0;
CREATE VIEW sieved_5 AS
SELECT n FROM sieved_3 WHERE n = 5 OR n % 5 != 0;
CREATE VIEW sieved_7 AS
SELECT n FROM sieved_5 WHERE n = 7 OR n % 7 != 0;
CREATE VIEW sieved_11 AS
SELECT n FROM sieved_7 WHERE n = 11 OR n % 11 != 0;
CREATE VIEW sieved_13 AS
SELECT n FROM sieved_11 WHERE n = 13 OR n % 13 != 0;
CREATE VIEW sieved_17 AS
SELECT n FROM sieved_13 WHERE n = 17 OR n % 17 != 0;
CREATE VIEW sieved_19 AS
SELECT n FROM sieved_17 WHERE n = 19 OR n % 19 != 0;
CREATE VIEW sieved_23 AS
SELECT n FROM sieved_19 WHERE n = 23 OR n % 23 != 0;
CREATE VIEW sieved_29 AS
SELECT n FROM sieved_23 WHERE n = 29 OR n % 29 != 0;
CREATE VIEW sieved_31 AS
SELECT n FROM sieved_29 WHERE n = 31 OR n % 31 != 0;
CREATE VIEW sieved_37 AS
SELECT n FROM sieved_31 WHERE n = 37 OR n % 37 != 0;
CREATE VIEW sieved_41 AS
SELECT n FROM sieved_37 WHERE n = 41 OR n % 41 != 0;
CREATE VIEW sieved_43 AS
SELECT n FROM sieved_41 WHERE n = 43 OR n % 43 != 0;
CREATE VIEW sieved_47 AS
SELECT n FROM sieved_43 WHERE n = 47 OR n % 47 != 0;
CREATE VIEW sieved_53 AS
SELECT n FROM sieved_47 WHERE n = 53 OR n % 53 != 0;
CREATE VIEW sieved_59 AS
SELECT n FROM sieved_53 WHERE n = 59 OR n % 59 != 0;
CREATE VIEW sieved_61 AS
SELECT n FROM sieved_59 WHERE n = 61 OR n % 61 != 0;
CREATE VIEW sieved_67 AS
SELECT n FROM sieved_61 WHERE n = 67 OR n % 67 != 0;
CREATE VIEW sieved_71 AS
SELECT n FROM sieved_67 WHERE n = 71 OR n % 71 != 0;
CREATE VIEW sieved_73 AS
SELECT n FROM sieved_71 WHERE n = 73 OR n % 73 != 0;
CREATE VIEW sieved_79 AS
SELECT n FROM sieved_73 WHERE n = 79 OR n % 79 != 0;
CREATE VIEW sieved_83 AS
SELECT n FROM sieved_79 WHERE n = 83 OR n % 83 != 0;
CREATE VIEW sieved_89 AS
SELECT n FROM sieved_83 WHERE n = 89 OR n % 89 != 0;
CREATE VIEW sieved_97 AS
SELECT n FROM sieved_89 WHERE n = 97 OR n % 97 != 0;

-- Output: count of primes discovered (pi(10^4) = 1229)
CREATE VIEW prime_count AS
SELECT count(*) AS n_primes FROM sieved_97;

EMIT INTO console SELECT * FROM prime_count;
