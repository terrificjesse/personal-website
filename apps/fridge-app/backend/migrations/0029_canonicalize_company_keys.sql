-- 0029 — canonicalize company keys, so one employer is one company.
--
-- GENERATED. Do not hand-edit; regenerate and re-review:
--
--     sqlite3 fridge.db ".backup '/tmp/company-candidates.db'"
--     COMPANY_FIXTURE_DB=/tmp/company-candidates.db \
--     COMPANY_MIGRATION_OUT=/tmp/0029_body.sql \
--       cargo test -p fridge_backend company_match -- --ignored --nocapture
--
-- The generator is `src/internships/company_match.rs` and it reads the same committed table
-- `normalize::company_key` reads at runtime, so this file and the running code cannot disagree
-- about what a company is called.
--
-- WHY, AND IT IS NOT WHAT THE TASK WAS FOR
--
-- 12d was raised to stop `KLA` and `KLA Corporation` being two rows. Measured on the live
-- corpus, that problem is already gone: `normalize::company_key` strips legal suffixes, so
-- every example § C names — KLA, Moog, WhatNot — is a single key today.
--
-- Applying the twenty-one reviewed aliases merges exactly **one** duplicate posting. That
-- alone would not justify a re-key over live data. What justifies it is a second effect
-- nobody was looking for: `company_signals` groups by `company_key`, and 19 companies had
-- their signal split across two or more keys, covering **130 postings**. Twelve of those
-- fragments carry no prestige at all, so their postings are scored at the neutral midpoint
-- while their sibling key scores real — `jump trading group` against `jump trading` at 1.0,
-- `drw university jobs` against `drw` at 0.88. Those postings are ranked as an average
-- company when the company is top-tier.
--
-- So this migration is a ranking fix that merges a duplicate on the way past.
--
-- SCOPE
--
--   47 postings get a new `company_key`
--    8 of those are fallback-keyed, so their `dedup_key` changes too
--   39 are ATS-keyed: `company_key` changes, `dedup_key` does not, no collision is possible
--    1 group collides and is merged, with no application and no alert on either row
--
-- WHAT IT REFUSES
--
-- `citadel securities` is NOT merged into `citadel`: different employers, one descriptive
-- token apart, and no string rule can tell them apart. Nor is the Rivian/Volkswagen joint
-- venture, nor `internship list`. The reasons live beside the decisions in
-- `data/internships/company-aliases.json`, because the next regeneration will propose all
-- three again and the reason not to merge is the part worth keeping.
--
-- THE DELETE GUARDS ALL THREE REFERENCES, WHICH 0025 DID NOT
--
-- A QC pass on 2026-09-03 found that 0025 guarded its DELETE on `internship_applications`
-- only, and orphaned a `hunt_events` row — `hunt_events.subject_id` is a soft reference and
-- not a declared foreign key, so `PRAGMA foreign_keys` did not catch it either. Three things
-- reference a posting: sightings, applications, and hunt_events. All three are guarded below,
-- and a repoint that would violate `UNIQUE (kind, subject_id)` drops the duplicate alert
-- rather than failing the migration.
--
-- On a fresh database none of these ids exist and the whole file is a no-op.

-- merge into ad6cfe30-b374-4fa9-be9b-d189b48d5dcf (co:susquehanna|quantitative-strategy-developer|summer-2027)

-- 7e12d90f-8e38-4852-94e6-157ca46011c2 -> ad6cfe30-b374-4fa9-be9b-d189b48d5dcf
UPDATE internship_applications SET posting_id = 'ad6cfe30-b374-4fa9-be9b-d189b48d5dcf'
  WHERE posting_id = '7e12d90f-8e38-4852-94e6-157ca46011c2'
    AND NOT EXISTS (SELECT 1 FROM internship_applications other
                     WHERE other.user_id = internship_applications.user_id
                       AND other.posting_id = 'ad6cfe30-b374-4fa9-be9b-d189b48d5dcf');
UPDATE posting_sightings SET posting_id = 'ad6cfe30-b374-4fa9-be9b-d189b48d5dcf' WHERE posting_id = '7e12d90f-8e38-4852-94e6-157ca46011c2';
DELETE FROM hunt_events WHERE subject_id = '7e12d90f-8e38-4852-94e6-157ca46011c2'
    AND EXISTS (SELECT 1 FROM hunt_events other
                 WHERE other.kind = hunt_events.kind
                   AND other.subject_id = 'ad6cfe30-b374-4fa9-be9b-d189b48d5dcf');
UPDATE hunt_events SET subject_id = 'ad6cfe30-b374-4fa9-be9b-d189b48d5dcf' WHERE subject_id = '7e12d90f-8e38-4852-94e6-157ca46011c2';
DELETE FROM internship_postings WHERE id = '7e12d90f-8e38-4852-94e6-157ca46011c2'
    AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = '7e12d90f-8e38-4852-94e6-157ca46011c2')
    AND NOT EXISTS (SELECT 1 FROM posting_sightings s WHERE s.posting_id = '7e12d90f-8e38-4852-94e6-157ca46011c2')
    AND NOT EXISTS (SELECT 1 FROM hunt_events h WHERE h.subject_id = '7e12d90f-8e38-4852-94e6-157ca46011c2');

-- re-key the survivors and every other renamed posting
UPDATE internship_postings SET company_key = 'varda space', dedup_key = 'ats:greenhouse:vardaspace:7824765003'
 WHERE id = '0180de01-9ecd-48b4-ac79-e88c97b79e9f';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:4668185007'
 WHERE id = '0a968814-c34d-4c78-b4bf-72bb53d30d34';
UPDATE internship_postings SET company_key = 'imc', dedup_key = 'ats:greenhouse:imc:4912874101'
 WHERE id = '0c58c7de-36ec-4854-a351-94ed2296e619';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:4968215007'
 WHERE id = '1507159c-105b-45c7-b1c8-fb5050d9eeff';
UPDATE internship_postings SET company_key = 'varda space', dedup_key = 'ats:greenhouse:vardaspace:7824814003'
 WHERE id = '152c6c84-77a4-4855-8772-c8bdc18c7382';
UPDATE internship_postings SET company_key = 'palantir', dedup_key = 'ats:lever:palantir:a483f41b-0da9-42ea-8ed6-cbf6eb93cc6d'
 WHERE id = '26509459-156d-4b93-9a26-8eaca210c324';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:4501164007'
 WHERE id = '283a7900-bcb9-40aa-9325-d0cea99cf5eb';
UPDATE internship_postings SET company_key = 'palantir', dedup_key = 'ats:lever:palantir:7d69cf8a-06fd-4f05-bd84-27149db29c4d'
 WHERE id = '2b4faff3-9785-4b47-aa54-2a6b8ff6d6f9';
UPDATE internship_postings SET company_key = 'drw', dedup_key = 'ats:greenhouse:drwuniversityjobs:7364884'
 WHERE id = '31f598ee-a839-482c-bd5d-81046f060225';
UPDATE internship_postings SET company_key = 'tmeic', dedup_key = 'ats:workable:tmeic-corporation-americas:532EE44DFB'
 WHERE id = '338325a9-ef91-4803-b67e-f65a165f66cb';
UPDATE internship_postings SET company_key = 'astera', dedup_key = 'ats:greenhouse:asteraearlycareer2026:4611422005'
 WHERE id = '33bdfe35-7821-45e5-9342-d57f41802815';
UPDATE internship_postings SET company_key = 'varda space', dedup_key = 'ats:greenhouse:vardaspace:7824766003'
 WHERE id = '36c12bd0-46c2-4585-ad2c-dc1b0b46dd7f';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:4501189007'
 WHERE id = '39e84b7b-f821-461a-8962-970539f9a5bc';
UPDATE internship_postings SET company_key = 'teledyne', dedup_key = 'ats:workday:flir.wd1:REQ36193'
 WHERE id = '44c6d0b5-3e6b-4934-a565-9263aa68ccd0';
UPDATE internship_postings SET company_key = 'gritt', dedup_key = 'ats:ashby:gritt:46af6e69-40fc-4e53-940e-a99757137523'
 WHERE id = '4c932e19-e69f-47fa-8da8-9cab9b870c14';
UPDATE internship_postings SET company_key = 'aquatic capital', dedup_key = 'ats:greenhouse:aquaticcapitalmanagement:8489233002'
 WHERE id = '51996d33-390d-4202-8110-980cdc77b7a2';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:5065140007'
 WHERE id = '531cc22e-cdea-4efa-b8ba-a46f03c52972';
UPDATE internship_postings SET company_key = 'voloridge', dedup_key = 'co:voloridge|quantitative-developer|summer-any'
 WHERE id = '5fb7fc7a-fc49-4ae0-8769-91d389f63110';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:5203134007'
 WHERE id = '60ebe445-9de4-442d-b24f-c7cebf2f7a76';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:5221670007'
 WHERE id = '6407ae31-b2e3-4b55-bf14-0c1928341b51';
UPDATE internship_postings SET company_key = 'arlo', dedup_key = 'ats:workday:arlo.wd12:JR100404'
 WHERE id = '65001e0b-97b4-423d-8c46-59adca38173a';
UPDATE internship_postings SET company_key = 'susquehanna', dedup_key = 'co:susquehanna|machine-learning|summer-2026'
 WHERE id = '65592139-d067-4064-9ad4-8be5c86e8c92';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:4968219007'
 WHERE id = '711843f7-c79a-4921-8cbb-5696d9b8c0cf';
UPDATE internship_postings SET company_key = 'gritt', dedup_key = 'ats:ashby:gritt:5c4737ce-f546-453b-b30d-791a121fb9fd'
 WHERE id = '787a07f9-7ad3-4420-9f56-3c6b0f284727';
UPDATE internship_postings SET company_key = 'susquehanna', dedup_key = 'co:susquehanna|quantitative-strategy-developer|summer-2027'
 WHERE id = '7e12d90f-8e38-4852-94e6-157ca46011c2';
UPDATE internship_postings SET company_key = 'varda space', dedup_key = 'ats:greenhouse:vardaspace:7824780003'
 WHERE id = '8e9db13e-91fa-44e8-97ef-5dda550c572d';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:4522665007'
 WHERE id = '8f9243a4-a2ea-4659-9138-269744b0cd54';
UPDATE internship_postings SET company_key = 'perplexity', dedup_key = 'ats:ashby:Perplexity:71168628-1998-47d3-87a9-be7bc56a430d'
 WHERE id = '8fc57f29-752f-4770-93a7-a95995421a6f';
UPDATE internship_postings SET company_key = 'anduril', dedup_key = 'ats:greenhouse:andurilindustries:5211077007'
 WHERE id = '94031a21-a673-4083-b516-6fbe223cbd7c';
UPDATE internship_postings SET company_key = 'susquehanna', dedup_key = 'co:susquehanna|trading-system-engineering|summer-2027'
 WHERE id = 'a001d75d-edab-4cb7-b4c8-341390645ab2';
UPDATE internship_postings SET company_key = 'tmeic', dedup_key = 'ats:workable:tmeic-corporation-americas:6FDBF2FD32'
 WHERE id = 'a8aad92d-a430-471c-beaa-f1aa87bf0be4';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:5165445007'
 WHERE id = 'a94bd298-2006-4b56-bf84-722cfab6823f';
UPDATE internship_postings SET company_key = 'susquehanna', dedup_key = 'co:susquehanna|quantitative-strategy-developer|summer-2027'
 WHERE id = 'ad6cfe30-b374-4fa9-be9b-d189b48d5dcf';
UPDATE internship_postings SET company_key = 'tenstorrent', dedup_key = 'ats:greenhouse:tenstorrentuniversity:5080887007'
 WHERE id = 'bac0186a-1fca-4fa4-9f03-d0dea0a5e3dd';
UPDATE internship_postings SET company_key = 'jump trading', dedup_key = 'ats:greenhouse:gh_jid:8003019'
 WHERE id = 'c454a02b-f179-4420-96a3-dbc1d06b95d9';
UPDATE internship_postings SET company_key = 'susquehanna', dedup_key = 'co:susquehanna|trading-systems-engineer|summer-2027'
 WHERE id = 'ce8fa6e1-b2c1-43ae-8bbf-a0583cebb82e';
UPDATE internship_postings SET company_key = 'smiths detection', dedup_key = 'ats:smartrecruiters:SmithsGroup2:744000145533267'
 WHERE id = 'd3253f49-5d05-4f84-9c54-7d18b8ffd4e1';
UPDATE internship_postings SET company_key = 'astera', dedup_key = 'ats:greenhouse:asteraearlycareer2026:4562833005'
 WHERE id = 'd570cfc2-4c12-44e3-90f8-9f6c59fd9584';
UPDATE internship_postings SET company_key = 'imc', dedup_key = 'ats:greenhouse:imc:4907430101'
 WHERE id = 'd9a34bd1-f62d-41c7-9713-781309ec13c6';
UPDATE internship_postings SET company_key = 'varda space', dedup_key = 'ats:greenhouse:vardaspace:7824817003'
 WHERE id = 'dbf20347-0e42-4a4f-bdc4-0bdf763a15d0';
UPDATE internship_postings SET company_key = 'imc', dedup_key = 'ats:greenhouse:imc:4842595101'
 WHERE id = 'e56d9fcb-be36-49f7-b275-fe652ee26833';
UPDATE internship_postings SET company_key = 'tmeic', dedup_key = 'ats:workable:tmeic-corporation-americas:68E556E5CA'
 WHERE id = 'e810753f-fd0b-415c-ad19-6630e340171b';
UPDATE internship_postings SET company_key = 'astera', dedup_key = 'ats:greenhouse:asteraearlycareer2026:4609356005'
 WHERE id = 'ea089467-3bf5-47be-b3fc-e85fa494e494';
UPDATE internship_postings SET company_key = 'nightwing', dedup_key = 'ats:workday:nwis.wd12:JR101095'
 WHERE id = 'edd2fdf3-9c23-4f3f-8ca0-b8714980bd0b';
UPDATE internship_postings SET company_key = 'susquehanna', dedup_key = 'co:susquehanna|quantitative-strategy-developer|summer-any'
 WHERE id = 'f0f69402-fa15-4740-8209-9e89744072ff';
UPDATE internship_postings SET company_key = 'procter gamble', dedup_key = 'ats:workday:pg.wd5:R000155305'
 WHERE id = 'fa2dc8ba-fb3f-41bc-8a7a-9531d7965bf0';
UPDATE internship_postings SET company_key = 'susquehanna', dedup_key = 'co:susquehanna|trading-system-engineer|summer-2027'
 WHERE id = 'fc928b80-6b6e-4aab-847a-c2ae8b08adb1';
