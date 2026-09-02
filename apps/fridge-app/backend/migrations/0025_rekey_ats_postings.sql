-- Re-key the postings 12a's parsers now identify, and merge the duplicates that creates
-- (Phase 12b).
--
-- # Why this file is a list of literal ids rather than logic
--
-- The new key is computed by `internships::dedup::ats_identity`, which is Rust. SQL cannot call
-- it. The alternative — a boot-time Rust routine that recomputes every key — would decide at
-- runtime what to merge in a database nobody had looked at. This file instead contains exactly
-- what was measured on 2026-09-02, and nothing else: **58 merge groups and 404 key rewrites**,
-- each naming its rows. It is auditable by reading it, and it cannot surprise anyone by merging
-- something that was not in the measurement.
--
-- Rows collected after 12a shipped already carry the correct key, so this list stays complete
-- however long it sits unapplied.
--
-- # Why it runs at all
--
-- `collector::upsert_posting` computes `dedup_key` and relies on `ON CONFLICT` to find the
-- existing row. 481 postings now compute a different key than the one stored against them, so
-- the next collection **inserts them as new rows** and lets the old ones expire — including the
-- posting the only application that reached OA points at. Doing nothing produces the same
-- merges anyway, via duplication and expiry, and without repointing anything.
--
-- # What it refuses to do
--
-- Two measured groups are NOT merged: `Tower Research` / `Tower Research Capital` and
-- `Nightwing` / `Nightwing Intelligence Solutions`. They share an ATS job id but disagree on
-- both company key and URL, so this file leaves them alone. They are the same employer under
-- two names, which is the fuzzy-matching problem 12d owns — and an unmerged duplicate is
-- cosmetic where a wrongly merged pair is two jobs becoming one, the failure Phase 7 caught
-- twice.
--
-- # Every statement is guarded, and degrades rather than failing
--
-- - An application is repointed only when the surviving posting does not already have one from
--   the same user — `UNIQUE (user_id, posting_id)` would otherwise abort the migration and take
--   the whole boot with it.
-- - A losing posting is deleted only when no application still points at it. If a repoint was
--   skipped, the loser survives with its old key: a duplicate row, which is the safe direction.
-- - A key rewrite is conditional on the stored key still being the one that was measured.
--
-- **Take a backup before the boot that runs this.** `ops/backup-fridge-db.sh`, and
-- `docs/DEPLOY.md` names it as a gate.

-- 1. Merge the duplicates.

-- ats:greenhouse:gh_jid:4830113101
UPDATE internship_applications SET posting_id = '1b108823-4777-4353-bd8c-1240753e1852'
 WHERE posting_id IN ('633bf4b7-434c-47a2-9374-0569715f57aa')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '1b108823-4777-4353-bd8c-1240753e1852');
UPDATE posting_sightings SET posting_id = '1b108823-4777-4353-bd8c-1240753e1852' WHERE posting_id IN ('633bf4b7-434c-47a2-9374-0569715f57aa');
DELETE FROM internship_postings WHERE id IN ('633bf4b7-434c-47a2-9374-0569715f57aa')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:5214784007
UPDATE internship_applications SET posting_id = '44c38c4f-6eae-4d30-b74f-20923ca78f90'
 WHERE posting_id IN ('b13fcce6-ffcb-45b5-b05c-8d8f5a75e637')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '44c38c4f-6eae-4d30-b74f-20923ca78f90');
UPDATE posting_sightings SET posting_id = '44c38c4f-6eae-4d30-b74f-20923ca78f90' WHERE posting_id IN ('b13fcce6-ffcb-45b5-b05c-8d8f5a75e637');
DELETE FROM internship_postings WHERE id IN ('b13fcce6-ffcb-45b5-b05c-8d8f5a75e637')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:5225258007
UPDATE internship_applications SET posting_id = 'bcc440ec-f971-4916-8753-1d2de50f78de'
 WHERE posting_id IN ('d9caca4c-1190-450b-a703-5469c3a1f72f')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'bcc440ec-f971-4916-8753-1d2de50f78de');
UPDATE posting_sightings SET posting_id = 'bcc440ec-f971-4916-8753-1d2de50f78de' WHERE posting_id IN ('d9caca4c-1190-450b-a703-5469c3a1f72f');
DELETE FROM internship_postings WHERE id IN ('d9caca4c-1190-450b-a703-5469c3a1f72f')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:5981740004
UPDATE internship_applications SET posting_id = '1a4a45f5-4e37-4625-8b7d-8a3e908901c7'
 WHERE posting_id IN ('b332c532-f797-4698-8a1d-c5c190369d1e')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '1a4a45f5-4e37-4625-8b7d-8a3e908901c7');
UPDATE posting_sightings SET posting_id = '1a4a45f5-4e37-4625-8b7d-8a3e908901c7' WHERE posting_id IN ('b332c532-f797-4698-8a1d-c5c190369d1e');
DELETE FROM internship_postings WHERE id IN ('b332c532-f797-4698-8a1d-c5c190369d1e')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:5987663004
UPDATE internship_applications SET posting_id = 'f6f74524-3ea2-4fbc-8f6f-895f283e4812'
 WHERE posting_id IN ('a93eda19-ddd8-44e1-8705-aba293c139b5')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'f6f74524-3ea2-4fbc-8f6f-895f283e4812');
UPDATE posting_sightings SET posting_id = 'f6f74524-3ea2-4fbc-8f6f-895f283e4812' WHERE posting_id IN ('a93eda19-ddd8-44e1-8705-aba293c139b5');
DELETE FROM internship_postings WHERE id IN ('a93eda19-ddd8-44e1-8705-aba293c139b5')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:6138134004
UPDATE internship_applications SET posting_id = 'bde82c6f-7a83-4e37-a5ce-11121894e2e9'
 WHERE posting_id IN ('65b162bb-ce25-46cc-81ba-78ab0bb4fb95')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'bde82c6f-7a83-4e37-a5ce-11121894e2e9');
UPDATE posting_sightings SET posting_id = 'bde82c6f-7a83-4e37-a5ce-11121894e2e9' WHERE posting_id IN ('65b162bb-ce25-46cc-81ba-78ab0bb4fb95');
DELETE FROM internship_postings WHERE id IN ('65b162bb-ce25-46cc-81ba-78ab0bb4fb95')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:6138140004
UPDATE internship_applications SET posting_id = 'f3f4f276-8bd4-403a-9e28-4edf72b64e01'
 WHERE posting_id IN ('400387a0-3d46-4c0a-a93c-e2dc4d9e958c', 'a58ec497-fb0c-48fb-bc81-d1f13b0bc218')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'f3f4f276-8bd4-403a-9e28-4edf72b64e01');
UPDATE posting_sightings SET posting_id = 'f3f4f276-8bd4-403a-9e28-4edf72b64e01' WHERE posting_id IN ('400387a0-3d46-4c0a-a93c-e2dc4d9e958c', 'a58ec497-fb0c-48fb-bc81-d1f13b0bc218');
DELETE FROM internship_postings WHERE id IN ('400387a0-3d46-4c0a-a93c-e2dc4d9e958c', 'a58ec497-fb0c-48fb-bc81-d1f13b0bc218')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:6141180004
UPDATE internship_applications SET posting_id = '1d0c8c6e-089a-4e3d-a0f1-315a0fa16b3d'
 WHERE posting_id IN ('5f986714-6ea0-490a-b630-aadb234a8b1f', 'a4a34ed2-7567-4a02-8033-00c0fdb45c33')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '1d0c8c6e-089a-4e3d-a0f1-315a0fa16b3d');
UPDATE posting_sightings SET posting_id = '1d0c8c6e-089a-4e3d-a0f1-315a0fa16b3d' WHERE posting_id IN ('5f986714-6ea0-490a-b630-aadb234a8b1f', 'a4a34ed2-7567-4a02-8033-00c0fdb45c33');
DELETE FROM internship_postings WHERE id IN ('5f986714-6ea0-490a-b630-aadb234a8b1f', 'a4a34ed2-7567-4a02-8033-00c0fdb45c33')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:6147167004
UPDATE internship_applications SET posting_id = 'fec4dbb7-1da8-415a-a09e-1ebf586d74b3'
 WHERE posting_id IN ('8a32b9b4-bfe0-486b-89a1-ea7654c5f115')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'fec4dbb7-1da8-415a-a09e-1ebf586d74b3');
UPDATE posting_sightings SET posting_id = 'fec4dbb7-1da8-415a-a09e-1ebf586d74b3' WHERE posting_id IN ('8a32b9b4-bfe0-486b-89a1-ea7654c5f115');
DELETE FROM internship_postings WHERE id IN ('8a32b9b4-bfe0-486b-89a1-ea7654c5f115')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:6147283004
UPDATE internship_applications SET posting_id = 'e463a123-b0bf-408f-a23b-bd33506ec4d2'
 WHERE posting_id IN ('d4d91327-ed4b-4f4d-a2a2-d436079957f6')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'e463a123-b0bf-408f-a23b-bd33506ec4d2');
UPDATE posting_sightings SET posting_id = 'e463a123-b0bf-408f-a23b-bd33506ec4d2' WHERE posting_id IN ('d4d91327-ed4b-4f4d-a2a2-d436079957f6');
DELETE FROM internship_postings WHERE id IN ('d4d91327-ed4b-4f4d-a2a2-d436079957f6')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:6152263004
UPDATE internship_applications SET posting_id = 'd3447a23-25be-4d1a-a883-39275de65217'
 WHERE posting_id IN ('684535c8-ccf2-42a2-b8d4-44222acab59d')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'd3447a23-25be-4d1a-a883-39275de65217');
UPDATE posting_sightings SET posting_id = 'd3447a23-25be-4d1a-a883-39275de65217' WHERE posting_id IN ('684535c8-ccf2-42a2-b8d4-44222acab59d');
DELETE FROM internship_postings WHERE id IN ('684535c8-ccf2-42a2-b8d4-44222acab59d')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:6163851004
UPDATE internship_applications SET posting_id = '06b381c3-bbd6-4eeb-85a3-dce64ffcdec0'
 WHERE posting_id IN ('9da03de9-080e-4b85-8bd3-00f6d72e581f')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '06b381c3-bbd6-4eeb-85a3-dce64ffcdec0');
UPDATE posting_sightings SET posting_id = '06b381c3-bbd6-4eeb-85a3-dce64ffcdec0' WHERE posting_id IN ('9da03de9-080e-4b85-8bd3-00f6d72e581f');
DELETE FROM internship_postings WHERE id IN ('9da03de9-080e-4b85-8bd3-00f6d72e581f')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:7255640
UPDATE internship_applications SET posting_id = '6939def3-6d37-4802-9d2a-8408e8074c68'
 WHERE posting_id IN ('2f3ef7ad-73a3-48ae-bd1f-c5bf0227c258')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '6939def3-6d37-4802-9d2a-8408e8074c68');
UPDATE posting_sightings SET posting_id = '6939def3-6d37-4802-9d2a-8408e8074c68' WHERE posting_id IN ('2f3ef7ad-73a3-48ae-bd1f-c5bf0227c258');
DELETE FROM internship_postings WHERE id IN ('2f3ef7ad-73a3-48ae-bd1f-c5bf0227c258')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:7258004
UPDATE internship_applications SET posting_id = '4a5746b4-e440-435f-a7d3-a1f664c2c90a'
 WHERE posting_id IN ('789f729a-c9b7-4c2e-b930-8ffc81099704')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '4a5746b4-e440-435f-a7d3-a1f664c2c90a');
UPDATE posting_sightings SET posting_id = '4a5746b4-e440-435f-a7d3-a1f664c2c90a' WHERE posting_id IN ('789f729a-c9b7-4c2e-b930-8ffc81099704');
DELETE FROM internship_postings WHERE id IN ('789f729a-c9b7-4c2e-b930-8ffc81099704')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:7796180003
UPDATE internship_applications SET posting_id = '61ea3ec0-979e-44c8-9fd0-00918bd79acd'
 WHERE posting_id IN ('1f82268d-1afc-42f7-bfad-b93afd304337', '74964d02-3576-439c-94df-b3a4db90cb9c')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '61ea3ec0-979e-44c8-9fd0-00918bd79acd');
UPDATE posting_sightings SET posting_id = '61ea3ec0-979e-44c8-9fd0-00918bd79acd' WHERE posting_id IN ('1f82268d-1afc-42f7-bfad-b93afd304337', '74964d02-3576-439c-94df-b3a4db90cb9c');
DELETE FROM internship_postings WHERE id IN ('1f82268d-1afc-42f7-bfad-b93afd304337', '74964d02-3576-439c-94df-b3a4db90cb9c')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:7886501003
UPDATE internship_applications SET posting_id = '440a8071-646e-4e41-b507-cff3123eb8c3'
 WHERE posting_id IN ('7e096d96-0da4-4b4f-b544-315db789ea56')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '440a8071-646e-4e41-b507-cff3123eb8c3');
UPDATE posting_sightings SET posting_id = '440a8071-646e-4e41-b507-cff3123eb8c3' WHERE posting_id IN ('7e096d96-0da4-4b4f-b544-315db789ea56');
DELETE FROM internship_postings WHERE id IN ('7e096d96-0da4-4b4f-b544-315db789ea56')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:7907191003
UPDATE internship_applications SET posting_id = '61c5da30-7643-47e9-822e-eff56c2607ef'
 WHERE posting_id IN ('6ce2efa0-be83-48cb-b680-8919d72b7f03')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '61c5da30-7643-47e9-822e-eff56c2607ef');
UPDATE posting_sightings SET posting_id = '61c5da30-7643-47e9-822e-eff56c2607ef' WHERE posting_id IN ('6ce2efa0-be83-48cb-b680-8919d72b7f03');
DELETE FROM internship_postings WHERE id IN ('6ce2efa0-be83-48cb-b680-8919d72b7f03')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:7908677003
UPDATE internship_applications SET posting_id = '537fb395-d364-4864-9fbb-d1fe346a7422'
 WHERE posting_id IN ('85e0e636-1dbb-43ad-b435-6c6db59632a7')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '537fb395-d364-4864-9fbb-d1fe346a7422');
UPDATE posting_sightings SET posting_id = '537fb395-d364-4864-9fbb-d1fe346a7422' WHERE posting_id IN ('85e0e636-1dbb-43ad-b435-6c6db59632a7');
DELETE FROM internship_postings WHERE id IN ('85e0e636-1dbb-43ad-b435-6c6db59632a7')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:7929236003
UPDATE internship_applications SET posting_id = 'dfbbbfb4-4a85-4e00-80e7-2c1b1c8c7408'
 WHERE posting_id IN ('d711c9ff-4c3d-4bf0-a416-79fcb6fc525c')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'dfbbbfb4-4a85-4e00-80e7-2c1b1c8c7408');
UPDATE posting_sightings SET posting_id = 'dfbbbfb4-4a85-4e00-80e7-2c1b1c8c7408' WHERE posting_id IN ('d711c9ff-4c3d-4bf0-a416-79fcb6fc525c');
DELETE FROM internship_postings WHERE id IN ('d711c9ff-4c3d-4bf0-a416-79fcb6fc525c')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:7974897003
UPDATE internship_applications SET posting_id = 'fe227012-5807-48b2-ac0c-dd206aba9032'
 WHERE posting_id IN ('99c17b03-085a-48c0-a41a-3688b51a75ac')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'fe227012-5807-48b2-ac0c-dd206aba9032');
UPDATE posting_sightings SET posting_id = 'fe227012-5807-48b2-ac0c-dd206aba9032' WHERE posting_id IN ('99c17b03-085a-48c0-a41a-3688b51a75ac');
DELETE FROM internship_postings WHERE id IN ('99c17b03-085a-48c0-a41a-3688b51a75ac')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8002989
UPDATE internship_applications SET posting_id = 'aeefc529-953c-4e2d-9101-deebe529982b'
 WHERE posting_id IN ('9ad9fe2f-bb98-400c-bba7-d0aeafee0ec8')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'aeefc529-953c-4e2d-9101-deebe529982b');
UPDATE posting_sightings SET posting_id = 'aeefc529-953c-4e2d-9101-deebe529982b' WHERE posting_id IN ('9ad9fe2f-bb98-400c-bba7-d0aeafee0ec8');
DELETE FROM internship_postings WHERE id IN ('9ad9fe2f-bb98-400c-bba7-d0aeafee0ec8')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8003019
UPDATE internship_applications SET posting_id = 'c454a02b-f179-4420-96a3-dbc1d06b95d9'
 WHERE posting_id IN ('6cb2ca43-2209-43e0-98f9-8debca811fb2')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'c454a02b-f179-4420-96a3-dbc1d06b95d9');
UPDATE posting_sightings SET posting_id = 'c454a02b-f179-4420-96a3-dbc1d06b95d9' WHERE posting_id IN ('6cb2ca43-2209-43e0-98f9-8debca811fb2');
DELETE FROM internship_postings WHERE id IN ('6cb2ca43-2209-43e0-98f9-8debca811fb2')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8007788
UPDATE internship_applications SET posting_id = 'c11d847a-f0e7-4d32-b7d0-b2d55fb51b9e'
 WHERE posting_id IN ('c699c4f0-18f1-4435-aa5a-bab8fc836b23')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'c11d847a-f0e7-4d32-b7d0-b2d55fb51b9e');
UPDATE posting_sightings SET posting_id = 'c11d847a-f0e7-4d32-b7d0-b2d55fb51b9e' WHERE posting_id IN ('c699c4f0-18f1-4435-aa5a-bab8fc836b23');
DELETE FROM internship_postings WHERE id IN ('c699c4f0-18f1-4435-aa5a-bab8fc836b23')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8018847
UPDATE internship_applications SET posting_id = 'dc171b09-54e2-436c-9eb2-23b3115ffe30'
 WHERE posting_id IN ('e4b633f8-a75f-4928-9d94-ff7006b17313')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'dc171b09-54e2-436c-9eb2-23b3115ffe30');
UPDATE posting_sightings SET posting_id = 'dc171b09-54e2-436c-9eb2-23b3115ffe30' WHERE posting_id IN ('e4b633f8-a75f-4928-9d94-ff7006b17313');
DELETE FROM internship_postings WHERE id IN ('e4b633f8-a75f-4928-9d94-ff7006b17313')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8018853
UPDATE internship_applications SET posting_id = '5f217ee7-8a74-4bd3-a4e8-2dd5f67407c4'
 WHERE posting_id IN ('6cdc17bc-f976-4510-af72-d6a28a06c345')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '5f217ee7-8a74-4bd3-a4e8-2dd5f67407c4');
UPDATE posting_sightings SET posting_id = '5f217ee7-8a74-4bd3-a4e8-2dd5f67407c4' WHERE posting_id IN ('6cdc17bc-f976-4510-af72-d6a28a06c345');
DELETE FROM internship_postings WHERE id IN ('6cdc17bc-f976-4510-af72-d6a28a06c345')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8018856
UPDATE internship_applications SET posting_id = '2c9ce9a7-02d8-4886-81a0-f58abb15783f'
 WHERE posting_id IN ('0697c04a-06ab-4787-b62e-5c03a5610a44')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '2c9ce9a7-02d8-4886-81a0-f58abb15783f');
UPDATE posting_sightings SET posting_id = '2c9ce9a7-02d8-4886-81a0-f58abb15783f' WHERE posting_id IN ('0697c04a-06ab-4787-b62e-5c03a5610a44');
DELETE FROM internship_postings WHERE id IN ('0697c04a-06ab-4787-b62e-5c03a5610a44')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8018886
UPDATE internship_applications SET posting_id = 'ada9da93-7b3e-4c10-b807-eecd1e7f874d'
 WHERE posting_id IN ('045a41f6-1516-40de-86e8-6be964757e97')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'ada9da93-7b3e-4c10-b807-eecd1e7f874d');
UPDATE posting_sightings SET posting_id = 'ada9da93-7b3e-4c10-b807-eecd1e7f874d' WHERE posting_id IN ('045a41f6-1516-40de-86e8-6be964757e97');
DELETE FROM internship_postings WHERE id IN ('045a41f6-1516-40de-86e8-6be964757e97')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8018893
UPDATE internship_applications SET posting_id = '4da1306a-69dc-401e-ad27-89958687a119'
 WHERE posting_id IN ('3df5d66d-0c60-4509-9b70-c98acd9653e9')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '4da1306a-69dc-401e-ad27-89958687a119');
UPDATE posting_sightings SET posting_id = '4da1306a-69dc-401e-ad27-89958687a119' WHERE posting_id IN ('3df5d66d-0c60-4509-9b70-c98acd9653e9');
DELETE FROM internship_postings WHERE id IN ('3df5d66d-0c60-4509-9b70-c98acd9653e9')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8052083
UPDATE internship_applications SET posting_id = 'd7ab422c-9b0e-4830-96e2-2ffec81c4449'
 WHERE posting_id IN ('ff69b603-f735-4e9f-a1a3-fadad2dbbef0')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'd7ab422c-9b0e-4830-96e2-2ffec81c4449');
UPDATE posting_sightings SET posting_id = 'd7ab422c-9b0e-4830-96e2-2ffec81c4449' WHERE posting_id IN ('ff69b603-f735-4e9f-a1a3-fadad2dbbef0');
DELETE FROM internship_postings WHERE id IN ('ff69b603-f735-4e9f-a1a3-fadad2dbbef0')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8052095
UPDATE internship_applications SET posting_id = '836e2c95-5ca7-4caf-ba90-81c2420122ca'
 WHERE posting_id IN ('35607a4c-5bf6-4922-8488-ee7f8f54f3e7', 'eb85a075-b9bf-437f-b2a0-78c46b1b8770')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '836e2c95-5ca7-4caf-ba90-81c2420122ca');
UPDATE posting_sightings SET posting_id = '836e2c95-5ca7-4caf-ba90-81c2420122ca' WHERE posting_id IN ('35607a4c-5bf6-4922-8488-ee7f8f54f3e7', 'eb85a075-b9bf-437f-b2a0-78c46b1b8770');
DELETE FROM internship_postings WHERE id IN ('35607a4c-5bf6-4922-8488-ee7f8f54f3e7', 'eb85a075-b9bf-437f-b2a0-78c46b1b8770')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8072713
UPDATE internship_applications SET posting_id = '61216a67-1b91-4a0b-adc0-f85a97fa1076'
 WHERE posting_id IN ('8126cab5-afd6-461b-864d-004430e29190', 'e482889e-8709-4935-a0de-1572a3e49311')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '61216a67-1b91-4a0b-adc0-f85a97fa1076');
UPDATE posting_sightings SET posting_id = '61216a67-1b91-4a0b-adc0-f85a97fa1076' WHERE posting_id IN ('8126cab5-afd6-461b-864d-004430e29190', 'e482889e-8709-4935-a0de-1572a3e49311');
DELETE FROM internship_postings WHERE id IN ('8126cab5-afd6-461b-864d-004430e29190', 'e482889e-8709-4935-a0de-1572a3e49311')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8082091
UPDATE internship_applications SET posting_id = 'cfdc6193-5885-4ae7-838e-07e9799d06a1'
 WHERE posting_id IN ('b0f9ada1-8487-4ce6-8a61-3af6b8735628', 'b92dcbe1-bac8-4e20-90dc-dff21a357f18')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'cfdc6193-5885-4ae7-838e-07e9799d06a1');
UPDATE posting_sightings SET posting_id = 'cfdc6193-5885-4ae7-838e-07e9799d06a1' WHERE posting_id IN ('b0f9ada1-8487-4ce6-8a61-3af6b8735628', 'b92dcbe1-bac8-4e20-90dc-dff21a357f18');
DELETE FROM internship_postings WHERE id IN ('b0f9ada1-8487-4ce6-8a61-3af6b8735628', 'b92dcbe1-bac8-4e20-90dc-dff21a357f18')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8082093
UPDATE internship_applications SET posting_id = 'e67d3e3a-9f4d-4f5b-8cc6-960cb779c69d'
 WHERE posting_id IN ('07004240-e301-4361-b7f7-c4b021b5a287')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'e67d3e3a-9f4d-4f5b-8cc6-960cb779c69d');
UPDATE posting_sightings SET posting_id = 'e67d3e3a-9f4d-4f5b-8cc6-960cb779c69d' WHERE posting_id IN ('07004240-e301-4361-b7f7-c4b021b5a287');
DELETE FROM internship_postings WHERE id IN ('07004240-e301-4361-b7f7-c4b021b5a287')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8130352
UPDATE internship_applications SET posting_id = '4c03b68c-afb2-4bf9-b12b-b66db80c057d'
 WHERE posting_id IN ('59d19a0a-81be-4f62-b63c-d5c38a7858b3')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '4c03b68c-afb2-4bf9-b12b-b66db80c057d');
UPDATE posting_sightings SET posting_id = '4c03b68c-afb2-4bf9-b12b-b66db80c057d' WHERE posting_id IN ('59d19a0a-81be-4f62-b63c-d5c38a7858b3');
DELETE FROM internship_postings WHERE id IN ('59d19a0a-81be-4f62-b63c-d5c38a7858b3')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8132641
UPDATE internship_applications SET posting_id = 'cf9eca64-3613-4991-ac99-5760af0a474c'
 WHERE posting_id IN ('ce565773-4b99-4607-b2bf-ed50abd7f767')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'cf9eca64-3613-4991-ac99-5760af0a474c');
UPDATE posting_sightings SET posting_id = 'cf9eca64-3613-4991-ac99-5760af0a474c' WHERE posting_id IN ('ce565773-4b99-4607-b2bf-ed50abd7f767');
DELETE FROM internship_postings WHERE id IN ('ce565773-4b99-4607-b2bf-ed50abd7f767')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8202342002
UPDATE internship_applications SET posting_id = '948bbd8d-f9b3-4be8-89f9-3403d687f033'
 WHERE posting_id IN ('30593d5e-d296-4f13-a046-35808b4c7fe5')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '948bbd8d-f9b3-4be8-89f9-3403d687f033');
UPDATE posting_sightings SET posting_id = '948bbd8d-f9b3-4be8-89f9-3403d687f033' WHERE posting_id IN ('30593d5e-d296-4f13-a046-35808b4c7fe5');
DELETE FROM internship_postings WHERE id IN ('30593d5e-d296-4f13-a046-35808b4c7fe5')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8402114002
UPDATE internship_applications SET posting_id = '61812585-50bf-4e60-b6b4-a60803b6d18c'
 WHERE posting_id IN ('da97bf91-07eb-4c87-8713-3de01c42a4ba')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '61812585-50bf-4e60-b6b4-a60803b6d18c');
UPDATE posting_sightings SET posting_id = '61812585-50bf-4e60-b6b4-a60803b6d18c' WHERE posting_id IN ('da97bf91-07eb-4c87-8713-3de01c42a4ba');
DELETE FROM internship_postings WHERE id IN ('da97bf91-07eb-4c87-8713-3de01c42a4ba')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8589868002
UPDATE internship_applications SET posting_id = '8217a14a-dfb3-444c-89c6-ad3ead0d1100'
 WHERE posting_id IN ('f4b218b0-dbc5-4992-a3d7-130d273559b5')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '8217a14a-dfb3-444c-89c6-ad3ead0d1100');
UPDATE posting_sightings SET posting_id = '8217a14a-dfb3-444c-89c6-ad3ead0d1100' WHERE posting_id IN ('f4b218b0-dbc5-4992-a3d7-130d273559b5');
DELETE FROM internship_postings WHERE id IN ('f4b218b0-dbc5-4992-a3d7-130d273559b5')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8626146002
UPDATE internship_applications SET posting_id = 'c25f647a-132b-4070-afb0-44574bd54867'
 WHERE posting_id IN ('c866878a-7193-471d-adcf-0b628b51602e')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'c25f647a-132b-4070-afb0-44574bd54867');
UPDATE posting_sightings SET posting_id = 'c25f647a-132b-4070-afb0-44574bd54867' WHERE posting_id IN ('c866878a-7193-471d-adcf-0b628b51602e');
DELETE FROM internship_postings WHERE id IN ('c866878a-7193-471d-adcf-0b628b51602e')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8639480002
UPDATE internship_applications SET posting_id = '9e98eccb-4e42-4010-a55d-36cab0f55552'
 WHERE posting_id IN ('10aef13f-4269-431b-95fc-cdaad1b00ef8')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '9e98eccb-4e42-4010-a55d-36cab0f55552');
UPDATE posting_sightings SET posting_id = '9e98eccb-4e42-4010-a55d-36cab0f55552' WHERE posting_id IN ('10aef13f-4269-431b-95fc-cdaad1b00ef8');
DELETE FROM internship_postings WHERE id IN ('10aef13f-4269-431b-95fc-cdaad1b00ef8')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8687981002
UPDATE internship_applications SET posting_id = '2160c051-42fd-4794-b0f3-48cef8cb16c9'
 WHERE posting_id IN ('552350fa-a5cf-4416-8065-69d97746d0b2')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '2160c051-42fd-4794-b0f3-48cef8cb16c9');
UPDATE posting_sightings SET posting_id = '2160c051-42fd-4794-b0f3-48cef8cb16c9' WHERE posting_id IN ('552350fa-a5cf-4416-8065-69d97746d0b2');
DELETE FROM internship_postings WHERE id IN ('552350fa-a5cf-4416-8065-69d97746d0b2')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8700980002
UPDATE internship_applications SET posting_id = 'c910c88c-0237-4e89-aa05-e31e6fde33cf'
 WHERE posting_id IN ('eed24356-02af-42d4-9350-1d050f49a0e0')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'c910c88c-0237-4e89-aa05-e31e6fde33cf');
UPDATE posting_sightings SET posting_id = 'c910c88c-0237-4e89-aa05-e31e6fde33cf' WHERE posting_id IN ('eed24356-02af-42d4-9350-1d050f49a0e0');
DELETE FROM internship_postings WHERE id IN ('eed24356-02af-42d4-9350-1d050f49a0e0')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8713435002
UPDATE internship_applications SET posting_id = 'f2f0f5b6-d19f-4275-ac77-2aef34878201'
 WHERE posting_id IN ('160cde32-171f-44df-ae9d-89ce88d2c418')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'f2f0f5b6-d19f-4275-ac77-2aef34878201');
UPDATE posting_sightings SET posting_id = 'f2f0f5b6-d19f-4275-ac77-2aef34878201' WHERE posting_id IN ('160cde32-171f-44df-ae9d-89ce88d2c418');
DELETE FROM internship_postings WHERE id IN ('160cde32-171f-44df-ae9d-89ce88d2c418')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8732364002
UPDATE internship_applications SET posting_id = 'de6449c7-32f1-4e8f-abf4-a185a332616e'
 WHERE posting_id IN ('15c62951-c215-48b9-b529-8c3bc6e7c030')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'de6449c7-32f1-4e8f-abf4-a185a332616e');
UPDATE posting_sightings SET posting_id = 'de6449c7-32f1-4e8f-abf4-a185a332616e' WHERE posting_id IN ('15c62951-c215-48b9-b529-8c3bc6e7c030');
DELETE FROM internship_postings WHERE id IN ('15c62951-c215-48b9-b529-8c3bc6e7c030')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:greenhouse:gh_jid:8755768002
UPDATE internship_applications SET posting_id = '33cb78d7-d01d-4c4a-a395-02fb4ad7e92b'
 WHERE posting_id IN ('7f7331df-7430-4871-afe3-9784cf98e8aa')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '33cb78d7-d01d-4c4a-a395-02fb4ad7e92b');
UPDATE posting_sightings SET posting_id = '33cb78d7-d01d-4c4a-a395-02fb4ad7e92b' WHERE posting_id IN ('7f7331df-7430-4871-afe3-9784cf98e8aa');
DELETE FROM internship_postings WHERE id IN ('7f7331df-7430-4871-afe3-9784cf98e8aa')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:rippling:spreeai:c52472cb-2671-45d7-b666-17196dc3df25
UPDATE internship_applications SET posting_id = '7ffa0ab3-c04a-474c-b47f-83ac85aea107'
 WHERE posting_id IN ('c9a99437-3461-48db-9df6-06e56ff08452')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '7ffa0ab3-c04a-474c-b47f-83ac85aea107');
UPDATE posting_sightings SET posting_id = '7ffa0ab3-c04a-474c-b47f-83ac85aea107' WHERE posting_id IN ('c9a99437-3461-48db-9df6-06e56ff08452');
DELETE FROM internship_postings WHERE id IN ('c9a99437-3461-48db-9df6-06e56ff08452')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:rippling:spreeai:d34aed29-7a11-4e37-b5bc-e9317f82f0b1
UPDATE internship_applications SET posting_id = '007c17f2-8453-470e-bc76-048b7732491c'
 WHERE posting_id IN ('869ecc8f-7c8b-40f4-ba08-23f43bd5a179')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '007c17f2-8453-470e-bc76-048b7732491c');
UPDATE posting_sightings SET posting_id = '007c17f2-8453-470e-bc76-048b7732491c' WHERE posting_id IN ('869ecc8f-7c8b-40f4-ba08-23f43bd5a179');
DELETE FROM internship_postings WHERE id IN ('869ecc8f-7c8b-40f4-ba08-23f43bd5a179')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workable:pony-dot-ai:BA5FFDBC71
UPDATE internship_applications SET posting_id = '335af14f-07e8-40d5-9920-9545fbfbbeb8'
 WHERE posting_id IN ('b964ae5f-64e9-4ebc-bcf5-69c9ec8a2521')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '335af14f-07e8-40d5-9920-9545fbfbbeb8');
UPDATE posting_sightings SET posting_id = '335af14f-07e8-40d5-9920-9545fbfbbeb8' WHERE posting_id IN ('b964ae5f-64e9-4ebc-bcf5-69c9ec8a2521');
DELETE FROM internship_postings WHERE id IN ('b964ae5f-64e9-4ebc-bcf5-69c9ec8a2521')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workday:amgen.wd1:R-249424
UPDATE internship_applications SET posting_id = '73165525-ca05-438c-95c3-b3304d955d37'
 WHERE posting_id IN ('ac5f2ddc-3a0f-4219-ade3-661f9ef54a3e')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '73165525-ca05-438c-95c3-b3304d955d37');
UPDATE posting_sightings SET posting_id = '73165525-ca05-438c-95c3-b3304d955d37' WHERE posting_id IN ('ac5f2ddc-3a0f-4219-ade3-661f9ef54a3e');
DELETE FROM internship_postings WHERE id IN ('ac5f2ddc-3a0f-4219-ade3-661f9ef54a3e')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workday:capitalone.wd12:R249013
UPDATE internship_applications SET posting_id = 'bdeacc5f-bd00-4133-907c-5d4e36d8b827'
 WHERE posting_id IN ('c94c9a0f-3526-4d01-8d0b-775358f2bb6b')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'bdeacc5f-bd00-4133-907c-5d4e36d8b827');
UPDATE posting_sightings SET posting_id = 'bdeacc5f-bd00-4133-907c-5d4e36d8b827' WHERE posting_id IN ('c94c9a0f-3526-4d01-8d0b-775358f2bb6b');
DELETE FROM internship_postings WHERE id IN ('c94c9a0f-3526-4d01-8d0b-775358f2bb6b')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workday:capitalone.wd12:R249015
UPDATE internship_applications SET posting_id = 'ccabdb42-a939-4c6c-acb5-2ab4db5ee463'
 WHERE posting_id IN ('dc2cf4aa-e43c-4533-ac18-fc194fb1b887')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'ccabdb42-a939-4c6c-acb5-2ab4db5ee463');
UPDATE posting_sightings SET posting_id = 'ccabdb42-a939-4c6c-acb5-2ab4db5ee463' WHERE posting_id IN ('dc2cf4aa-e43c-4533-ac18-fc194fb1b887');
DELETE FROM internship_postings WHERE id IN ('dc2cf4aa-e43c-4533-ac18-fc194fb1b887')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workday:capitalone.wd12:R249022
UPDATE internship_applications SET posting_id = '676b232b-10c4-4521-91b0-196e664bff66'
 WHERE posting_id IN ('7349aca9-c1cd-4754-82d9-31d8c8eed16c')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '676b232b-10c4-4521-91b0-196e664bff66');
UPDATE posting_sightings SET posting_id = '676b232b-10c4-4521-91b0-196e664bff66' WHERE posting_id IN ('7349aca9-c1cd-4754-82d9-31d8c8eed16c');
DELETE FROM internship_postings WHERE id IN ('7349aca9-c1cd-4754-82d9-31d8c8eed16c')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workday:coreandmain.wd1:45804
UPDATE internship_applications SET posting_id = '31aa88e6-d45a-4c7b-93c0-e041266f03ca'
 WHERE posting_id IN ('60937c65-ad4e-453c-9e71-4bdbda64baef')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '31aa88e6-d45a-4c7b-93c0-e041266f03ca');
UPDATE posting_sightings SET posting_id = '31aa88e6-d45a-4c7b-93c0-e041266f03ca' WHERE posting_id IN ('60937c65-ad4e-453c-9e71-4bdbda64baef');
DELETE FROM internship_postings WHERE id IN ('60937c65-ad4e-453c-9e71-4bdbda64baef')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workday:crowe.wd12:R-71041
UPDATE internship_applications SET posting_id = '41646c4c-f8b9-4149-bacf-e29782c666a4'
 WHERE posting_id IN ('c69866fc-d3ac-4ad1-84ff-c7e129a4dfce')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '41646c4c-f8b9-4149-bacf-e29782c666a4');
UPDATE posting_sightings SET posting_id = '41646c4c-f8b9-4149-bacf-e29782c666a4' WHERE posting_id IN ('c69866fc-d3ac-4ad1-84ff-c7e129a4dfce');
DELETE FROM internship_postings WHERE id IN ('c69866fc-d3ac-4ad1-84ff-c7e129a4dfce')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workday:leidos.wd5:R-00189691
UPDATE internship_applications SET posting_id = '7c3ef191-e361-486d-a0e8-2a98c14e5a3c'
 WHERE posting_id IN ('d39d72c8-b07f-4d92-9ac2-7a8dae0a356a')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '7c3ef191-e361-486d-a0e8-2a98c14e5a3c');
UPDATE posting_sightings SET posting_id = '7c3ef191-e361-486d-a0e8-2a98c14e5a3c' WHERE posting_id IN ('d39d72c8-b07f-4d92-9ac2-7a8dae0a356a');
DELETE FROM internship_postings WHERE id IN ('d39d72c8-b07f-4d92-9ac2-7a8dae0a356a')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workday:monolithicpower.wd12:R-890
UPDATE internship_applications SET posting_id = '3ea00598-eebf-432f-97e5-e701b54ea854'
 WHERE posting_id IN ('62a732c1-a0b6-4cec-96f2-da33feda8c77')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = '3ea00598-eebf-432f-97e5-e701b54ea854');
UPDATE posting_sightings SET posting_id = '3ea00598-eebf-432f-97e5-e701b54ea854' WHERE posting_id IN ('62a732c1-a0b6-4cec-96f2-da33feda8c77');
DELETE FROM internship_postings WHERE id IN ('62a732c1-a0b6-4cec-96f2-da33feda8c77')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workday:osv-cci.wd1:R1346
UPDATE internship_applications SET posting_id = 'b1c098a2-d9a4-418a-ae0d-d6fb58d2cdf7'
 WHERE posting_id IN ('c81a1752-146c-40f7-9c07-0b7aa7b0736a')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'b1c098a2-d9a4-418a-ae0d-d6fb58d2cdf7');
UPDATE posting_sightings SET posting_id = 'b1c098a2-d9a4-418a-ae0d-d6fb58d2cdf7' WHERE posting_id IN ('c81a1752-146c-40f7-9c07-0b7aa7b0736a');
DELETE FROM internship_postings WHERE id IN ('c81a1752-146c-40f7-9c07-0b7aa7b0736a')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- ats:workday:osv-cci.wd1:R1350
UPDATE internship_applications SET posting_id = 'd8736927-c88b-4ed9-9400-d5fafae4df0e'
 WHERE posting_id IN ('f8e65d2a-0354-4d06-a8e8-22bfaa4d12af')
   AND NOT EXISTS (SELECT 1 FROM internship_applications other
                    WHERE other.user_id = internship_applications.user_id
                      AND other.posting_id = 'd8736927-c88b-4ed9-9400-d5fafae4df0e');
UPDATE posting_sightings SET posting_id = 'd8736927-c88b-4ed9-9400-d5fafae4df0e' WHERE posting_id IN ('f8e65d2a-0354-4d06-a8e8-22bfaa4d12af');
DELETE FROM internship_postings WHERE id IN ('f8e65d2a-0354-4d06-a8e8-22bfaa4d12af')
   AND NOT EXISTS (SELECT 1 FROM internship_applications a WHERE a.posting_id = internship_postings.id);

-- 2. Release every key that is about to change, so nothing claims a key another row still
--    holds. `dedup_key` is UNIQUE, and a rewrite order that happens to work on one database
--    is not a property anyone can rely on.
UPDATE internship_postings SET dedup_key = 'rekey-0025:7ffa0ab3-c04a-474c-b47f-83ac85aea107' WHERE id = '7ffa0ab3-c04a-474c-b47f-83ac85aea107' AND dedup_key = 'co:spreeai|mobile-software-engineer|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:007c17f2-8453-470e-bc76-048b7732491c' WHERE id = '007c17f2-8453-470e-bc76-048b7732491c' AND dedup_key = 'co:spreeai|software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:335af14f-07e8-40d5-9920-9545fbfbbeb8' WHERE id = '335af14f-07e8-40d5-9920-9545fbfbbeb8' AND dedup_key = 'co:pony ai|software-engineer|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:73165525-ca05-438c-95c3-b3304d955d37' WHERE id = '73165525-ca05-438c-95c3-b3304d955d37' AND dedup_key = 'co:amgen|software-engineer|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:bdeacc5f-bd00-4133-907c-5d4e36d8b827' WHERE id = 'bdeacc5f-bd00-4133-907c-5d4e36d8b827' AND dedup_key = 'co:capital one|full-stack-software-engineer-team-pickle|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ccabdb42-a939-4c6c-acb5-2ab4db5ee463' WHERE id = 'ccabdb42-a939-4c6c-acb5-2ab4db5ee463' AND dedup_key = 'co:capital one|software-engineer-mobile|winter-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:676b232b-10c4-4521-91b0-196e664bff66' WHERE id = '676b232b-10c4-4521-91b0-196e664bff66' AND dedup_key = 'co:capital one|backend-software-engineer-team-interstellar|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:31aa88e6-d45a-4c7b-93c0-e041266f03ca' WHERE id = '31aa88e6-d45a-4c7b-93c0-e041266f03ca' AND dedup_key = 'co:core main|data-engineering|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:41646c4c-f8b9-4149-bacf-e29782c666a4' WHERE id = '41646c4c-f8b9-4149-bacf-e29782c666a4' AND dedup_key = 'co:crowe|data-analytics-developer-consulting-practice|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7c3ef191-e361-486d-a0e8-2a98c14e5a3c' WHERE id = '7c3ef191-e361-486d-a0e8-2a98c14e5a3c' AND dedup_key = 'co:leidos|engineering|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3ea00598-eebf-432f-97e5-e701b54ea854' WHERE id = '3ea00598-eebf-432f-97e5-e701b54ea854' AND dedup_key = 'co:monolithic power systems|application-engineer|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b1c098a2-d9a4-418a-ae0d-d6fb58d2cdf7' WHERE id = 'b1c098a2-d9a4-418a-ae0d-d6fb58d2cdf7' AND dedup_key = 'co:castleton commodities international|data-engineering|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d8736927-c88b-4ed9-9400-d5fafae4df0e' WHERE id = 'd8736927-c88b-4ed9-9400-d5fafae4df0e' AND dedup_key = 'co:castleton commodities international|full-stack-software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:62ca9c5c-5da2-4f74-9620-0585be3e83e0' WHERE id = '62ca9c5c-5da2-4f74-9620-0585be3e83e0' AND dedup_key = 'co:aerovironment|software-engineering-hyper-rf-division|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f52f4dce-dfd7-4d2a-8738-104ef5ac1fde' WHERE id = 'f52f4dce-dfd7-4d2a-8738-104ef5ac1fde' AND dedup_key = 'co:marvell|applied-machine-learning-scientist-phd|spring-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6cb1657d-47cd-4feb-9360-04442f366608' WHERE id = '6cb1657d-47cd-4feb-9360-04442f366608' AND dedup_key = 'co:ge appliances|software-engineering-co-op|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:02dc0d35-a80e-46d2-b1be-212c16aa13c2' WHERE id = '02dc0d35-a80e-46d2-b1be-212c16aa13c2' AND dedup_key = 'co:aptiv|engineering|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:337331d5-276e-4459-9d80-b71b2e780e25' WHERE id = '337331d5-276e-4459-9d80-b71b2e780e25' AND dedup_key = 'co:expedia group|machine-learning-science|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3443738d-ff70-4a96-8a86-149c1ebea8c5' WHERE id = '3443738d-ff70-4a96-8a86-149c1ebea8c5' AND dedup_key = 'co:cadence design systems|software|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b9315d46-c801-48df-8279-f771eb22c1ea' WHERE id = 'b9315d46-c801-48df-8279-f771eb22c1ea' AND dedup_key = 'co:pennstate university|research-and-development-engineering|spring-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2323b70f-37d6-44cf-a8e3-89f8154fcc72' WHERE id = '2323b70f-37d6-44cf-a8e3-89f8154fcc72' AND dedup_key = 'co:marmon holdings|data-engineering-co-op|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:651ac0b4-8210-453e-91ba-9be5bf5c354d' WHERE id = '651ac0b4-8210-453e-91ba-9be5bf5c354d' AND dedup_key = 'co:marmon holdings|digital-production-engineering-or-student-co-op|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:617f1110-515f-4986-a652-dea958c7aba1' WHERE id = '617f1110-515f-4986-a652-dea958c7aba1' AND dedup_key = 'co:occidental petroleum|co-op-data-well-servicing-engineering|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:edd2fdf3-9c23-4f3f-8ca0-b8714980bd0b' WHERE id = 'edd2fdf3-9c23-4f3f-8ca0-b8714980bd0b' AND dedup_key = 'co:nightwing intelligence solutions|radio-frequency-engineering|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:38b835f8-2d5f-4e83-8d0b-56db76441e42' WHERE id = '38b835f8-2d5f-4e83-8d0b-56db76441e42' AND dedup_key = 'co:hewlett packard hp|browser-software-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ff57b238-d736-4763-a3f2-40ce59609b95' WHERE id = 'ff57b238-d736-4763-a3f2-40ce59609b95' AND dedup_key = 'co:arlo|sw-engineer-ios|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6b8f6d3d-ed92-4f8a-a5d2-c36042364ba7' WHERE id = '6b8f6d3d-ed92-4f8a-a5d2-c36042364ba7' AND dedup_key = 'co:arlo|sw-engineer-android|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6a076fe6-b50e-4f4b-aebc-a2fd28251c61' WHERE id = '6a076fe6-b50e-4f4b-aebc-a2fd28251c61' AND dedup_key = 'co:applied materials|physics-ai-modeling-engineering|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:503bd406-9309-4bce-a26b-0204619660e2' WHERE id = '503bd406-9309-4bce-a26b-0204619660e2' AND dedup_key = 'co:nio|llm-algorithmic-optimization-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2e19ed93-bd38-4057-aebe-22d3c8de7e6b' WHERE id = '2e19ed93-bd38-4057-aebe-22d3c8de7e6b' AND dedup_key = 'co:thermo fisher scientific|engineering-co-op|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:394656c8-ba01-4e42-95be-4cdbfc43f364' WHERE id = '394656c8-ba01-4e42-95be-4cdbfc43f364' AND dedup_key = 'co:omnis|software-engineering-co-op|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:40f820fb-278f-4196-9f69-a8772adde15b' WHERE id = '40f820fb-278f-4196-9f69-a8772adde15b' AND dedup_key = 'co:tencent|software-engineering-pc-game-client-development|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:76fb57ed-a9e7-43df-a35c-c3d34366207e' WHERE id = '76fb57ed-a9e7-43df-a35c-c3d34366207e' AND dedup_key = 'co:corpay|software-developer-co-op|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2a7245ca-271c-4afa-a55a-5966abd2da46' WHERE id = '2a7245ca-271c-4afa-a55a-5966abd2da46' AND dedup_key = 'co:intel|ai-software-engineering|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:4960ecb1-b6ab-42c5-b593-73963f8888de' WHERE id = '4960ecb1-b6ab-42c5-b593-73963f8888de' AND dedup_key = 'co:tmeic|applications-ai-and-machine-learning|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b9759705-5f64-4d84-aa73-707053cf053c' WHERE id = 'b9759705-5f64-4d84-aa73-707053cf053c' AND dedup_key = 'co:kla|software-engineering|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:39d0be1a-e745-4bf2-bf96-5f3396bb1d2c' WHERE id = '39d0be1a-e745-4bf2-bf96-5f3396bb1d2c' AND dedup_key = 'co:eluvio|ai-machine-learning-gen-ai-multimodal|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d7f2cafb-488e-4bc9-a5eb-979a8435a1d8' WHERE id = 'd7f2cafb-488e-4bc9-a5eb-979a8435a1d8' AND dedup_key = 'co:cisive|software-development|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:fc556629-5774-4ee2-b580-20bebae33251' WHERE id = 'fc556629-5774-4ee2-b580-20bebae33251' AND dedup_key = 'co:cae|engineering-co-op|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:296394cc-8c29-42a5-bc0a-f34d24a4bd97' WHERE id = '296394cc-8c29-42a5-bc0a-f34d24a4bd97' AND dedup_key = 'co:first quality|analytics-engineer-co-op-analytics-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b4dc3766-66d3-421d-81de-69f6c4727e6a' WHERE id = 'b4dc3766-66d3-421d-81de-69f6c4727e6a' AND dedup_key = 'co:jade global|data-ai-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e1e021e2-ea71-4e88-9c7a-13b2435abc2c' WHERE id = 'e1e021e2-ea71-4e88-9c7a-13b2435abc2c' AND dedup_key = 'co:menasha|application-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:781b3382-f1b0-4b6d-8dc9-eef84361aac7' WHERE id = '781b3382-f1b0-4b6d-8dc9-eef84361aac7' AND dedup_key = 'co:magna|product-engineering-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:8797a520-c360-49e0-9c72-f6d08e92011b' WHERE id = '8797a520-c360-49e0-9c72-f6d08e92011b' AND dedup_key = 'co:marvell|applied-machine-learning-scientist|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:1e6833c7-ca7b-4729-9f0c-79fefa404109' WHERE id = '1e6833c7-ca7b-4729-9f0c-79fefa404109' AND dedup_key = 'co:magna|junior-full-stack-developer-co-op|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:be01b8c4-57bd-4ac4-8dab-c0a710365ca9' WHERE id = 'be01b8c4-57bd-4ac4-8dab-c0a710365ca9' AND dedup_key = 'co:altom transport|software-development|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:60681084-fd67-4d40-be40-36d7aa0e4541' WHERE id = '60681084-fd67-4d40-be40-36d7aa0e4541' AND dedup_key = 'co:rippling|machine-learning-software-engineer|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e6a328d3-7c0e-4af9-8c13-ee7a7a82b988' WHERE id = 'e6a328d3-7c0e-4af9-8c13-ee7a7a82b988' AND dedup_key = 'co:copart|software-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d71cdf89-f425-4ffb-a219-218083431972' WHERE id = 'd71cdf89-f425-4ffb-a219-218083431972' AND dedup_key = 'co:intel|ai-software-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:405ade5a-0bcd-47f2-b8d3-ee1e36258650' WHERE id = '405ade5a-0bcd-47f2-b8d3-ee1e36258650' AND dedup_key = 'co:the campbell s|modeling-and-visualization-engineer-co-op|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:5d236eab-47c9-4073-829d-43c0b577f61a' WHERE id = '5d236eab-47c9-4073-829d-43c0b577f61a' AND dedup_key = 'co:the campbell s|data-engineer-co-op|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:be26d2d9-73a7-4318-af81-ad3e867dc5b3' WHERE id = 'be26d2d9-73a7-4318-af81-ad3e867dc5b3' AND dedup_key = 'co:synchrony financial|software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:53f08158-0c42-4057-855c-f7a23ba6cb85' WHERE id = '53f08158-0c42-4057-855c-f7a23ba6cb85' AND dedup_key = 'co:the campbell s|data-engineer-co-op-operational-support|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:93d015c4-bed6-43dc-809b-f4f107461935' WHERE id = '93d015c4-bed6-43dc-809b-f4f107461935' AND dedup_key = 'co:the campbell s|agentic-ai-engineer-co-op|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:bd5b0fb9-a560-411e-be46-72354e506727' WHERE id = 'bd5b0fb9-a560-411e-be46-72354e506727' AND dedup_key = 'co:viridien|software-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:aa673bf5-b7b0-4b4b-afc3-d4f59da1ab79' WHERE id = 'aa673bf5-b7b0-4b4b-afc3-d4f59da1ab79' AND dedup_key = 'co:sony|software-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:363c06c9-9298-4a37-9e31-b30b0709fcbe' WHERE id = '363c06c9-9298-4a37-9e31-b30b0709fcbe' AND dedup_key = 'co:nelnet|ai-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:1e491cd2-24d3-4abf-8c28-1fbf676435fd' WHERE id = '1e491cd2-24d3-4abf-8c28-1fbf676435fd' AND dedup_key = 'co:copart|database-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f20c8e21-89c9-4d71-8ae6-3263c2b0b5a4' WHERE id = 'f20c8e21-89c9-4d71-8ae6-3263c2b0b5a4' AND dedup_key = 'co:palo alto networks|software-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e6fb1aab-3ef8-4006-a7ba-d677280c7aee' WHERE id = 'e6fb1aab-3ef8-4006-a7ba-d677280c7aee' AND dedup_key = 'co:palo alto networks|software-engineer|summer-2025';
UPDATE internship_postings SET dedup_key = 'rekey-0025:4d64f71a-68c6-4d0e-a161-7092470ba03b' WHERE id = '4d64f71a-68c6-4d0e-a161-7092470ba03b' AND dedup_key = 'co:magna|computer-vision-engineering-co-op|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:56655e51-345b-4f47-9a40-1516ee5af4fc' WHERE id = '56655e51-345b-4f47-9a40-1516ee5af4fc' AND dedup_key = 'co:hitachi|software-analyst|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:023b5314-fbde-410b-804c-a7842f4e5ae6' WHERE id = '023b5314-fbde-410b-804c-a7842f4e5ae6' AND dedup_key = 'co:arrowstreet capital|quantitative-developer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:fa2dc8ba-fb3f-41bc-8a7a-9531d7965bf0' WHERE id = 'fa2dc8ba-fb3f-41bc-8a7a-9531d7965bf0' AND dedup_key = 'co:procter gamble p g|r-d-engineer-co-op|winter-2028';
UPDATE internship_postings SET dedup_key = 'rekey-0025:87b8c126-e7fe-429f-8bcc-ea60500ea07a' WHERE id = '87b8c126-e7fe-429f-8bcc-ea60500ea07a' AND dedup_key = 'co:revvity|front-end-ai-marketing-co-op|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:66d9e680-fe24-4be7-be6c-627d6352a9fd' WHERE id = '66d9e680-fe24-4be7-be6c-627d6352a9fd' AND dedup_key = 'co:ge aerospace|engineer|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c8c7ca59-1c14-45c0-8de6-a931c46c2e97' WHERE id = 'c8c7ca59-1c14-45c0-8de6-a931c46c2e97' AND dedup_key = 'co:ge aerospace|unison-engineer|fall-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c6b1411e-cf87-44dd-852e-200133c110f3' WHERE id = 'c6b1411e-cf87-44dd-852e-200133c110f3' AND dedup_key = 'co:copart|software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2c09ffb6-aed8-4a49-89d6-1bf192bb496f' WHERE id = '2c09ffb6-aed8-4a49-89d6-1bf192bb496f' AND dedup_key = 'co:solar turbines|gas-turbine-products-engineering|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:23a77ccd-da2a-4e89-8a1a-f2d03b0544b5' WHERE id = '23a77ccd-da2a-4e89-8a1a-f2d03b0544b5' AND dedup_key = 'co:chevron|software-engineer-information-technology-software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:edc63891-7dd5-4c1b-b943-b1063fc89404' WHERE id = 'edc63891-7dd5-4c1b-b943-b1063fc89404' AND dedup_key = 'co:ensemble health partners|engineering-excellence|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:176a4755-dac5-4af2-b49f-2213a0aaccb7' WHERE id = '176a4755-dac5-4af2-b49f-2213a0aaccb7' AND dedup_key = 'co:magna|ai-engineering-co-op|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2af51f42-a59d-43db-aac2-eaf939612133' WHERE id = '2af51f42-a59d-43db-aac2-eaf939612133' AND dedup_key = 'co:denari|product-software|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:5354c43d-acdf-4353-ac6d-a3a036e5273d' WHERE id = '5354c43d-acdf-4353-ac6d-a3a036e5273d' AND dedup_key = 'co:onware|full-stack-developer-opportunity|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:18f23050-605c-473f-a89e-9c1691f0fd89' WHERE id = '18f23050-605c-473f-a89e-9c1691f0fd89' AND dedup_key = 'co:spreeai|machine-learning-engineer-computer-vision-multimodal-generative-ai|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2000b0aa-0d1a-4760-85cf-6d7b764f4db9' WHERE id = '2000b0aa-0d1a-4760-85cf-6d7b764f4db9' AND dedup_key = 'co:gitar|software-engineer|summer-2025';
UPDATE internship_postings SET dedup_key = 'rekey-0025:4a7dfbcd-f80e-447e-b749-005d6cbc977d' WHERE id = '4a7dfbcd-f80e-447e-b749-005d6cbc977d' AND dedup_key = 'co:teledyne|computer-engineer|spring-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e1fa10dd-e30b-4ecc-92e8-0fef63b76ad8' WHERE id = 'e1fa10dd-e30b-4ecc-92e8-0fef63b76ad8' AND dedup_key = 'co:ambarella|mixed-signal-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d1e0b402-1734-48a2-b602-92ca589c857c' WHERE id = 'd1e0b402-1734-48a2-b602-92ca589c857c' AND dedup_key = 'co:marvell|analog-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a92f9c30-3378-4707-98c1-22ca37af4f78' WHERE id = 'a92f9c30-3378-4707-98c1-22ca37af4f78' AND dedup_key = 'co:ge aerospace|embedded-systems-engineer-co-op|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b8c7c92a-3dd7-4a3a-ad5d-d3f1560036ab' WHERE id = 'b8c7c92a-3dd7-4a3a-ad5d-d3f1560036ab' AND dedup_key = 'co:the boeing|to-entry-level-conversion-engineering|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:33c98a9b-268c-4e9a-b160-6c041f6a6b94' WHERE id = '33c98a9b-268c-4e9a-b160-6c041f6a6b94' AND dedup_key = 'co:microchip technology|equipment-engineering-technician-metrology|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:116f29e7-92a8-46aa-850c-afb370b5ed47' WHERE id = '116f29e7-92a8-46aa-850c-afb370b5ed47' AND dedup_key = 'co:nidec|test-lab-engineer-co-op|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d20bb200-a808-403a-be7e-024e6297078f' WHERE id = 'd20bb200-a808-403a-be7e-024e6297078f' AND dedup_key = 'co:castleton commodities international|front-office-software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d393454d-ddb8-4cb2-8465-312a99131386' WHERE id = 'd393454d-ddb8-4cb2-8465-312a99131386' AND dedup_key = 'co:castleton commodities international|data-science-machine-learning|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9e76a493-9f87-4754-8cdf-5adef8ee71ea' WHERE id = '9e76a493-9f87-4754-8cdf-5adef8ee71ea' AND dedup_key = 'co:g research|machine-learning-research|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3a4ccaec-4cdb-4856-9915-a192e1cd01dc' WHERE id = '3a4ccaec-4cdb-4856-9915-a192e1cd01dc' AND dedup_key = 'co:magna|r-d-computer-vision-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7cfd63b6-f6e5-4b8e-a8ff-4c7c447a8f58' WHERE id = '7cfd63b6-f6e5-4b8e-a8ff-4c7c447a8f58' AND dedup_key = 'co:pennsylvania state university|research-engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e118b30a-682c-419e-b7a3-98e88ea922c2' WHERE id = 'e118b30a-682c-419e-b7a3-98e88ea922c2' AND dedup_key = 'co:ambarella|asic-design-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3c594b30-149e-43db-b06f-03c5e68cc558' WHERE id = '3c594b30-149e-43db-b06f-03c5e68cc558' AND dedup_key = 'co:ambarella|verification-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e95953ea-f630-423f-b196-f52d99ed5fe1' WHERE id = 'e95953ea-f630-423f-b196-f52d99ed5fe1' AND dedup_key = 'co:ambarella|software-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:aeada356-563c-4ad3-b7b7-682598c759aa' WHERE id = 'aeada356-563c-4ad3-b7b7-682598c759aa' AND dedup_key = 'co:axis capital|renewable-energy-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:52fd037b-5f52-498d-b8df-1a7138a59834' WHERE id = '52fd037b-5f52-498d-b8df-1a7138a59834' AND dedup_key = 'co:magna|iot-systems-engineer-co-op|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d8c33668-72ed-456b-9508-5de3fbca4922' WHERE id = 'd8c33668-72ed-456b-9508-5de3fbca4922' AND dedup_key = 'co:aptiv|engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:392efca9-0296-45cd-b811-45fb02d606b0' WHERE id = '392efca9-0296-45cd-b811-45fb02d606b0' AND dedup_key = 'co:lightguide|application-engineering-co-op|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:50a6159d-4dc0-4e45-a7ea-6dee4cd2d830' WHERE id = '50a6159d-4dc0-4e45-a7ea-6dee4cd2d830' AND dedup_key = 'co:marmon holdings|digital-production-engineer-co-op|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b7822c23-671a-4eec-b132-779630ea44cb' WHERE id = 'b7822c23-671a-4eec-b132-779630ea44cb' AND dedup_key = 'co:microchip technology|equipment-engineering-technician-wet-process|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:938d2397-8d81-4f99-abff-f3b25631ad8d' WHERE id = '938d2397-8d81-4f99-abff-f3b25631ad8d' AND dedup_key = 'co:university of nevada reno|web-development-marketing-communications|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9e152168-e7cc-4b0b-93c6-1ff97993e1bf' WHERE id = '9e152168-e7cc-4b0b-93c6-1ff97993e1bf' AND dedup_key = 'co:hendrick motorsports|project-and-race-support-engineer|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2aa8a486-104e-4dc0-9368-9b1813a1bc9c' WHERE id = '2aa8a486-104e-4dc0-9368-9b1813a1bc9c' AND dedup_key = 'co:medtronic|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:52661293-1fe9-48e3-bb00-2a4236e8412e' WHERE id = '52661293-1fe9-48e3-bb00-2a4236e8412e' AND dedup_key = 'co:ciena|wavelogic-software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:972fc179-8a8a-4d89-8566-0d40938ba308' WHERE id = '972fc179-8a8a-4d89-8566-0d40938ba308' AND dedup_key = 'co:novanta|engineering-co-op|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7c2bbdda-7f8c-4ff7-8905-7c5f56d41e90' WHERE id = '7c2bbdda-7f8c-4ff7-8905-7c5f56d41e90' AND dedup_key = 'co:novanta|engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:bb8fd547-4994-422a-9600-cd6a12d50c24' WHERE id = 'bb8fd547-4994-422a-9600-cd6a12d50c24' AND dedup_key = 'co:uline|business-intelligence-developer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e16606e2-c732-400a-ab56-a58598208013' WHERE id = 'e16606e2-c732-400a-ab56-a58598208013' AND dedup_key = 'co:uline|software-development|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:aa807a96-76ed-4ea7-8f79-f9fc6c1de6d9' WHERE id = 'aa807a96-76ed-4ea7-8f79-f9fc6c1de6d9' AND dedup_key = 'co:microchip technology|engineering-software-development|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:93f71ba0-ee1e-4a31-b2cb-1eff60040968' WHERE id = '93f71ba0-ee1e-4a31-b2cb-1eff60040968' AND dedup_key = 'co:e careers|software-developer-trainee|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:8786006c-cf5b-496b-ae78-a82ee6a33db2' WHERE id = '8786006c-cf5b-496b-ae78-a82ee6a33db2' AND dedup_key = 'co:nidec|engineering-co-op|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:65001e0b-97b4-423d-8c46-59adca38173a' WHERE id = '65001e0b-97b4-423d-8c46-59adca38173a' AND dedup_key = 'co:arlo technologies|firmware-developer-co-op|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:612b1a18-9949-44ba-a248-162e7f7760fe' WHERE id = '612b1a18-9949-44ba-a248-162e7f7760fe' AND dedup_key = 'co:ge appliances|engineering-co-op|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7e9f8453-26a9-4db7-9ffd-f4762800543e' WHERE id = '7e9f8453-26a9-4db7-9ffd-f4762800543e' AND dedup_key = 'co:ge appliances|software-engineer-co-op-software-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:8b2484bd-b047-44ba-b28e-bef9d53e0554' WHERE id = '8b2484bd-b047-44ba-b28e-bef9d53e0554' AND dedup_key = 'co:northrop grumman|software-engineer-aeronautics-systems|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c05f57a4-8119-4dd9-9ccc-0bfa6139c0ff' WHERE id = 'c05f57a4-8119-4dd9-9ccc-0bfa6139c0ff' AND dedup_key = 'co:schweitzer engineering laboratories|application-engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9ec2f6cd-3995-4ee3-b44b-ee7ad53b86a2' WHERE id = '9ec2f6cd-3995-4ee3-b44b-ee7ad53b86a2' AND dedup_key = 'co:luminance|ai-engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:bb097eea-c221-4421-b843-850af0c02d12' WHERE id = 'bb097eea-c221-4421-b843-850af0c02d12' AND dedup_key = 'co:ciena|software-developer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d1dd7318-920b-4a2a-b059-153d3ff44473' WHERE id = 'd1dd7318-920b-4a2a-b059-153d3ff44473' AND dedup_key = 'co:rtx|systems-engineer-1-conversion|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:036f438d-a732-4e74-bea2-fdb40e95ebf2' WHERE id = '036f438d-a732-4e74-bea2-fdb40e95ebf2' AND dedup_key = 'co:amcor|product-development-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:8589d864-545a-4875-9951-68561a6d71aa' WHERE id = '8589d864-545a-4875-9951-68561a6d71aa' AND dedup_key = 'co:trillium|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f8657107-e829-40e2-828b-a0607ac89aba' WHERE id = 'f8657107-e829-40e2-828b-a0607ac89aba' AND dedup_key = 'co:autodesk|cloud-developer-interactive-graphics-media-entertainment|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6cc3cd64-826f-4480-b6ba-48a67ba88a1f' WHERE id = '6cc3cd64-826f-4480-b6ba-48a67ba88a1f' AND dedup_key = 'co:autodesk|cloud-developer-fcap|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d1820183-6213-48bc-b879-5999c444110e' WHERE id = 'd1820183-6213-48bc-b879-5999c444110e' AND dedup_key = 'co:pronexus|software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e63afb76-2f37-4a79-a54c-f386ce3888f0' WHERE id = 'e63afb76-2f37-4a79-a54c-f386ce3888f0' AND dedup_key = 'co:booz allen|ai-ran-telecommunications-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:0d43f402-f060-46b2-9f16-f46d444461b0' WHERE id = '0d43f402-f060-46b2-9f16-f46d444461b0' AND dedup_key = 'co:lpl financial holdings|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:033a421e-ed3d-4d23-ba0f-0704938c9190' WHERE id = '033a421e-ed3d-4d23-ba0f-0704938c9190' AND dedup_key = 'co:lpl financial holdings|data-engineer-data|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3ebfc8b8-393a-4284-a774-ee52bd843243' WHERE id = '3ebfc8b8-393a-4284-a774-ee52bd843243' AND dedup_key = 'co:caddi workflow automation|software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b63134ea-00a7-4c0b-84af-c2789c8b0f12' WHERE id = 'b63134ea-00a7-4c0b-84af-c2789c8b0f12' AND dedup_key = 'co:responsiveads|full-stack-developer-responsiveads-studio-4|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:94e430d1-2eaa-4bd0-b125-f0ccd67af3a6' WHERE id = '94e430d1-2eaa-4bd0-b125-f0ccd67af3a6' AND dedup_key = 'co:micron technology|dram-ip-circuits-design-engineer-ip-development|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3b22dd22-db7b-4ded-8732-2934ca26e2b6' WHERE id = '3b22dd22-db7b-4ded-8732-2934ca26e2b6' AND dedup_key = 'co:micron technology|dram-design-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9b96bd34-9652-45e3-a7f4-355e3e07541d' WHERE id = '9b96bd34-9652-45e3-a7f4-355e3e07541d' AND dedup_key = 'co:autodesk|software-developer-interactive-graphics-media-entertainment|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a5ef2631-5ee3-4d43-ae47-a26ed56a1496' WHERE id = 'a5ef2631-5ee3-4d43-ae47-a26ed56a1496' AND dedup_key = 'co:autodesk|software-developer|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:fb5b75a2-094d-4156-9c83-52a8da162b30' WHERE id = 'fb5b75a2-094d-4156-9c83-52a8da162b30' AND dedup_key = 'co:quantbot technologies|quantitative-developer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:00f5a0eb-c6b7-4947-b032-4e61d1eab869' WHERE id = '00f5a0eb-c6b7-4947-b032-4e61d1eab869' AND dedup_key = 'co:quantbot technologies|machine-learning-research-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:323eba0f-e8f2-4334-980f-557aa710ffd5' WHERE id = '323eba0f-e8f2-4334-980f-557aa710ffd5' AND dedup_key = 'co:rtx|software-development|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d6047404-5634-407b-a133-09f151397e55' WHERE id = 'd6047404-5634-407b-a133-09f151397e55' AND dedup_key = 'co:blue origin|software-development-engineer-1-corporate-functions|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e810753f-fd0b-415c-ad19-6630e340171b' WHERE id = 'e810753f-fd0b-415c-ad19-6630e340171b' AND dedup_key = 'co:tmeic corporation americas|engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a8aad92d-a430-471c-beaa-f1aa87bf0be4' WHERE id = 'a8aad92d-a430-471c-beaa-f1aa87bf0be4' AND dedup_key = 'co:tmeic corporation americas|applications-ai-and-machine-learning|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:10be6784-c1d1-4671-a2c2-b98652c4e36a' WHERE id = '10be6784-c1d1-4671-a2c2-b98652c4e36a' AND dedup_key = 'co:american fidelity|software-development|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:0d3549ae-b826-469f-8a99-a59ccaaa1524' WHERE id = '0d3549ae-b826-469f-8a99-a59ccaaa1524' AND dedup_key = 'co:schweitzer engineering laboratories|software-engineer-ai-focus|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:051431cd-7a40-4b3b-8bbd-3b86f64e6117' WHERE id = '051431cd-7a40-4b3b-8bbd-3b86f64e6117' AND dedup_key = 'co:schweitzer engineering laboratories|engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:71572d77-c807-45cf-9dc9-43bcc921590a' WHERE id = '71572d77-c807-45cf-9dc9-43bcc921590a' AND dedup_key = 'co:micron technology|digital-ip-design-engineer-dram|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:fc1148ef-ad2f-4e02-a224-76de2a6d45fa' WHERE id = 'fc1148ef-ad2f-4e02-a224-76de2a6d45fa' AND dedup_key = 'co:crowe|ai-engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d0d8967b-2abd-4c4c-9761-9847d6c5879c' WHERE id = 'd0d8967b-2abd-4c4c-9761-9847d6c5879c' AND dedup_key = 'co:teledyne|software-engineer-nhrc|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7d76d7fb-2d40-4444-acf5-572eb8924736' WHERE id = '7d76d7fb-2d40-4444-acf5-572eb8924736' AND dedup_key = 'co:baker hughes|benefit-tool-developer-month-fixed-term-contract|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:abd2a4b0-ef92-45b2-8b04-d8fd73f5ee6a' WHERE id = 'abd2a4b0-ef92-45b2-8b04-d8fd73f5ee6a' AND dedup_key = 'co:generac|firmware-engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:36b5c5c0-1e50-4be6-aefe-919447275458' WHERE id = '36b5c5c0-1e50-4be6-aefe-919447275458' AND dedup_key = 'co:motorola|android-platform-software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:5e5cc234-2f69-4bf9-959f-ac7ef7348c15' WHERE id = '5e5cc234-2f69-4bf9-959f-ac7ef7348c15' AND dedup_key = 'co:intel|physical-design-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:8a3b10ee-aaf4-4690-8c86-932e71d46bba' WHERE id = '8a3b10ee-aaf4-4690-8c86-932e71d46bba' AND dedup_key = 'co:interdigital|wireless-engineering-6g-wireless-systems|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:90235642-e064-4846-bbc1-34f8f4f46fbd' WHERE id = '90235642-e064-4846-bbc1-34f8f4f46fbd' AND dedup_key = 'co:oneok|engineering|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:84c8bf16-7159-4b62-a1e5-c016286f4e53' WHERE id = '84c8bf16-7159-4b62-a1e5-c016286f4e53' AND dedup_key = 'co:crowe|machine-learning|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:5b56a4b6-a845-4e12-8971-0350a548d4d2' WHERE id = '5b56a4b6-a845-4e12-8971-0350a548d4d2' AND dedup_key = 'co:boom supersonic|engineering-and-tech|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:79e60b2a-762b-4ddc-aefa-9fd4591031a1' WHERE id = '79e60b2a-762b-4ddc-aefa-9fd4591031a1' AND dedup_key = 'co:nxp semiconductors|ai-and-software-engineer-automotive-mpus|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:03dd05c3-751a-42b5-84bb-3ec2dcec6d3d' WHERE id = '03dd05c3-751a-42b5-84bb-3ec2dcec6d3d' AND dedup_key = 'co:ge aerospace|aerospace-engineering-engines-co-op-computer-or-software-engineering|fall-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2c3dee96-ac3d-449e-b012-b573959e1772' WHERE id = '2c3dee96-ac3d-449e-b012-b573959e1772' AND dedup_key = 'co:suncor|automation-software-or-computer-engineering-student|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:0e26bf56-32c8-43e2-bad4-d09add5608b7' WHERE id = '0e26bf56-32c8-43e2-bad4-d09add5608b7' AND dedup_key = 'co:capital one|full-stack-software-engineer-team-integrated-sprout|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:61225403-6321-4ccb-a6c3-8acf224da672' WHERE id = '61225403-6321-4ccb-a6c3-8acf224da672' AND dedup_key = 'co:dee zee|software-development|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e900c485-7392-4f64-a4d5-185c8382c782' WHERE id = 'e900c485-7392-4f64-a4d5-185c8382c782' AND dedup_key = 'co:american fidelity|software-mobile|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:07822e9c-aa5a-4684-9cc2-7e375562a802' WHERE id = '07822e9c-aa5a-4684-9cc2-7e375562a802' AND dedup_key = 'co:royal bank of canada|developer-co-op-multiple-teams|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:697b1cd2-6776-48c5-a7db-d95d2f6bfa08' WHERE id = '697b1cd2-6776-48c5-a7db-d95d2f6bfa08' AND dedup_key = 'co:royal bank of canada|machine-learning-software-engineer-co-op-4-months|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:fabdba89-b6db-474e-9506-89eff3e6460d' WHERE id = 'fabdba89-b6db-474e-9506-89eff3e6460d' AND dedup_key = 'co:royal bank of canada|capital-markets-quantitative-technology-services-co-op-software-developer|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:5575d8d2-4bc9-4f53-8e40-91f9dc6dbc50' WHERE id = '5575d8d2-4bc9-4f53-8e40-91f9dc6dbc50' AND dedup_key = 'co:royal bank of canada|technology-operations-co-op-software-developer|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:bc52bac6-2467-475e-8a16-0b3989d5bd56' WHERE id = 'bc52bac6-2467-475e-8a16-0b3989d5bd56' AND dedup_key = 'co:royal bank of canada|software-developer-co-op-quantitative-technology-services|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e0a82643-7d09-47bb-a347-33c56f4ab6bd' WHERE id = 'e0a82643-7d09-47bb-a347-33c56f4ab6bd' AND dedup_key = 'co:royal bank of canada|machine-learning-software-engineer-co-op-rbc-borealis|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:8286b617-5020-455c-bd64-13a3feb6e561' WHERE id = '8286b617-5020-455c-bd64-13a3feb6e561' AND dedup_key = 'co:royal bank of canada|software-developer-co-op-technology-operations|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:418834cf-1a1e-4855-819b-87078c72192f' WHERE id = '418834cf-1a1e-4855-819b-87078c72192f' AND dedup_key = 'co:royal bank of canada|developer-co-op-technology-operations|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:021075d5-0792-48de-a18b-338fdf2bdc62' WHERE id = '021075d5-0792-48de-a18b-338fdf2bdc62' AND dedup_key = 'co:intel|software-development|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a369e9ac-8462-4378-aca3-1ccb4270af43' WHERE id = 'a369e9ac-8462-4378-aca3-1ccb4270af43' AND dedup_key = 'co:royal bank of canada|quantitative-technology-services-co-op-software-developer|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6fc84751-2310-4f42-8a4f-9796d1910b91' WHERE id = '6fc84751-2310-4f42-8a4f-9796d1910b91' AND dedup_key = 'co:royal bank of canada|machine-learning-software-engineer-co-op-multiple-teams|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3cdf5694-2ddd-4d62-85a1-c0da0046476a' WHERE id = '3cdf5694-2ddd-4d62-85a1-c0da0046476a' AND dedup_key = 'co:royal bank of canada|technology-and-operations-developer-co-op-software-developer|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b389bb41-7ce1-4af3-bb25-d379421eb44f' WHERE id = 'b389bb41-7ce1-4af3-bb25-d379421eb44f' AND dedup_key = 'co:royal bank of canada|quantitative-technology-services-co-op-qts-software-developer|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c161a7b6-c1f4-450e-8c36-8e4d7c16bd62' WHERE id = 'c161a7b6-c1f4-450e-8c36-8e4d7c16bd62' AND dedup_key = 'co:royal bank of canada|developer-co-op-multiple-roles|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d1ad8c66-5fc9-410b-9704-5b7e0e2eaa54' WHERE id = 'd1ad8c66-5fc9-410b-9704-5b7e0e2eaa54' AND dedup_key = 'co:rtx|conversion-systems-engineer-1|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c973db42-1962-408a-a53b-2e38d1633413' WHERE id = 'c973db42-1962-408a-a53b-2e38d1633413' AND dedup_key = 'co:rtx|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:35041534-47c7-4a88-90bb-505503e80efa' WHERE id = '35041534-47c7-4a88-90bb-505503e80efa' AND dedup_key = 'co:rtx|software-engineer-intelligent-software-systems|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:0a3a2701-2766-48d8-8f5b-f7b820fbb651' WHERE id = '0a3a2701-2766-48d8-8f5b-f7b820fbb651' AND dedup_key = 'co:rtx|software-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:02784837-f744-4ff0-ba35-4ae38c0ba9ba' WHERE id = '02784837-f744-4ff0-ba35-4ae38c0ba9ba' AND dedup_key = 'co:draper|full-stack-web-development-co-op|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:05bca639-db14-4fe1-ad3e-cc7ac2d6c70e' WHERE id = '05bca639-db14-4fe1-ad3e-cc7ac2d6c70e' AND dedup_key = 'co:foresters financial|software-engineer-co-op-ai|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9cc995a4-abd2-41cf-8b26-14851ed7284b' WHERE id = '9cc995a4-abd2-41cf-8b26-14851ed7284b' AND dedup_key = 'co:amcor|ai-innovation-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9a5ba322-f3fb-4055-9527-b8614c330dbf' WHERE id = '9a5ba322-f3fb-4055-9527-b8614c330dbf' AND dedup_key = 'co:pimco|software-engineering-technology-analyst|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b1dad54a-12b6-4da6-8865-bfb183e0fbf5' WHERE id = 'b1dad54a-12b6-4da6-8865-bfb183e0fbf5' AND dedup_key = 'co:nvidia|systems-software-engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f9eb9f37-49a3-4995-a7dc-235f56e019e1' WHERE id = 'f9eb9f37-49a3-4995-a7dc-235f56e019e1' AND dedup_key = 'co:nvidia|developer-and-performance-technology|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:24ac88e4-c7fc-45fe-911e-6a21c530a2c2' WHERE id = '24ac88e4-c7fc-45fe-911e-6a21c530a2c2' AND dedup_key = 'co:nvidia|software-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f644d175-0027-4e71-9fe2-9d54eb2052d1' WHERE id = 'f644d175-0027-4e71-9fe2-9d54eb2052d1' AND dedup_key = 'co:nvidia|ph-d-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2b70a559-694b-423d-b29c-d95869ca38e1' WHERE id = '2b70a559-694b-423d-b29c-d95869ca38e1' AND dedup_key = 'co:copart|data-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3e973e33-55d6-43ef-a8dd-841744be2c1c' WHERE id = '3e973e33-55d6-43ef-a8dd-841744be2c1c' AND dedup_key = 'co:ge vernova|application-engineer-co-op-pcs|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b1915506-fb01-4c6a-8c94-0b59cf847e5f' WHERE id = 'b1915506-fb01-4c6a-8c94-0b59cf847e5f' AND dedup_key = 'co:moog|computer-science-information-technology|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d9b21fc8-10e9-46c4-b3d5-fa0a9a18dd9a' WHERE id = 'd9b21fc8-10e9-46c4-b3d5-fa0a9a18dd9a' AND dedup_key = 'co:autodesk|ai-developer|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:0ea56dcd-72f2-4a5c-b18b-ae2f896d79fb' WHERE id = '0ea56dcd-72f2-4a5c-b18b-ae2f896d79fb' AND dedup_key = 'co:autodesk|ai-developer-creative-technology|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:338325a9-ef91-4803-b67e-f65a165f66cb' WHERE id = '338325a9-ef91-4803-b67e-f65a165f66cb' AND dedup_key = 'co:tmeic corporation americas|engineer|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:17cbd4f4-7426-44f5-a4c4-f867000fb8b9' WHERE id = '17cbd4f4-7426-44f5-a4c4-f867000fb8b9' AND dedup_key = 'co:western magnetics|software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c80e9d8d-0b0b-4ad5-a5c0-138cfde724d3' WHERE id = 'c80e9d8d-0b0b-4ad5-a5c0-138cfde724d3' AND dedup_key = 'co:ampersand|data-engineering-co-op-open-to-northeastern-students-only|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:23b73715-0fde-407b-89eb-416252685cb0' WHERE id = '23b73715-0fde-407b-89eb-416252685cb0' AND dedup_key = 'co:sysco|software-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e99496e1-6680-4830-a9e6-8e85c1239353' WHERE id = 'e99496e1-6680-4830-a9e6-8e85c1239353' AND dedup_key = 'co:fifth third bank|software-engineer-co-op-enterprise-finance-applications|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:0dc5d729-c295-4507-8d4d-0ead2b538939' WHERE id = '0dc5d729-c295-4507-8d4d-0ead2b538939' AND dedup_key = 'co:geico|technology-development-ai-engineer-development-track|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2563d96d-1f63-4deb-8bfe-6bf86750744f' WHERE id = '2563d96d-1f63-4deb-8bfe-6bf86750744f' AND dedup_key = 'co:geico|technology-development-software-engineer-development-track|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a7487f11-2dcc-44c2-892a-33c240adab21' WHERE id = 'a7487f11-2dcc-44c2-892a-33c240adab21' AND dedup_key = 'co:sysco|data-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7fd687d8-5ec3-4529-aeeb-4f11f33531de' WHERE id = '7fd687d8-5ec3-4529-aeeb-4f11f33531de' AND dedup_key = 'co:devon energy|technology-data-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6feef66b-4331-46f1-abdc-bcb0ec4c4a97' WHERE id = '6feef66b-4331-46f1-abdc-bcb0ec4c4a97' AND dedup_key = 'co:excellus bcbs|college-ai-engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a55eb371-1514-4c6b-8ab8-433239b6688f' WHERE id = 'a55eb371-1514-4c6b-8ab8-433239b6688f' AND dedup_key = 'co:excellus bcbs|software-engineering-multiple-openings|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e6a732d2-1532-4080-8ebc-c1d3a5c2c841' WHERE id = 'e6a732d2-1532-4080-8ebc-c1d3a5c2c841' AND dedup_key = 'co:eversource energy|asset-management-technology-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:acedd991-1657-4883-ada0-d60834d9668c' WHERE id = 'acedd991-1657-4883-ada0-d60834d9668c' AND dedup_key = 'co:micron technology|soc-rtl-design-engineer-hbm|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:0066a591-40c8-45b0-8527-91d29dbeda0d' WHERE id = '0066a591-40c8-45b0-8527-91d29dbeda0d' AND dedup_key = 'co:micron technology|physical-failure-analysis-engineer-yield-enhancement|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b009f355-954f-472a-9031-aabc41bfafec' WHERE id = 'b009f355-954f-472a-9031-aabc41bfafec' AND dedup_key = 'co:analog devices|mixed-signal-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ea1a034c-79c0-4532-b344-8475294482e2' WHERE id = 'ea1a034c-79c0-4532-b344-8475294482e2' AND dedup_key = 'co:ontario teachers pension plan|portfolio-engineer-capital-markets-cmia|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a602301e-83d6-406d-b4ca-972d199a71c2' WHERE id = 'a602301e-83d6-406d-b4ca-972d199a71c2' AND dedup_key = 'co:rippling|frontend-software-engineer|winter-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d4c25f40-dd0a-40ba-9c7c-2352aca539d7' WHERE id = 'd4c25f40-dd0a-40ba-9c7c-2352aca539d7' AND dedup_key = 'co:rippling|machine-learning-engineer|winter-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6578318f-23d7-48ac-a4ba-aa88ec455aa6' WHERE id = '6578318f-23d7-48ac-a4ba-aa88ec455aa6' AND dedup_key = 'co:rippling|software-engineer|winter-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:50be4f72-f56b-46c9-bc13-e3f5091b9a7b' WHERE id = '50be4f72-f56b-46c9-bc13-e3f5091b9a7b' AND dedup_key = 'co:al warren oil|software-developer|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:39d12781-895a-48b3-92ad-3ff0593dfe58' WHERE id = '39d12781-895a-48b3-92ad-3ff0593dfe58' AND dedup_key = 'co:abb|application-engineering|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3fc863a0-b4fd-4e24-9597-af00347400fc' WHERE id = '3fc863a0-b4fd-4e24-9597-af00347400fc' AND dedup_key = 'co:moon|software-engineer-backend-api|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6d0f3b3a-3dd0-443d-a4f6-24dd612f8a0a' WHERE id = '6d0f3b3a-3dd0-443d-a4f6-24dd612f8a0a' AND dedup_key = 'co:mobius renewables|software-engineer|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f0b73ea4-dda6-4f4c-8263-e16fb96619b6' WHERE id = 'f0b73ea4-dda6-4f4c-8263-e16fb96619b6' AND dedup_key = 'co:hyperlight|software-engineer|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:275b145e-e12e-4b33-83a9-2f16547c8eb8' WHERE id = '275b145e-e12e-4b33-83a9-2f16547c8eb8' AND dedup_key = 'co:castleton commodities international|data-science-machine-learning|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:454f395b-deba-43a5-9d54-739bd0abf4eb' WHERE id = '454f395b-deba-43a5-9d54-739bd0abf4eb' AND dedup_key = 'co:medtronic|software-engineering|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d67999ed-0b2f-4df2-bf0c-7722935a4983' WHERE id = 'd67999ed-0b2f-4df2-bf0c-7722935a4983' AND dedup_key = 'co:capital one|software-engineer|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d8bab602-37da-4a9f-af07-216fad703ece' WHERE id = 'd8bab602-37da-4a9f-af07-216fad703ece' AND dedup_key = 'co:netsmart|software-engineer|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:693e1bc8-8b39-4845-9a12-fd80839b3dcd' WHERE id = '693e1bc8-8b39-4845-9a12-fd80839b3dcd' AND dedup_key = 'co:rtx|software-engineer-fleet-health-instrumentation|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6eca1169-369b-4682-9bdb-5179b1c41e16' WHERE id = '6eca1169-369b-4682-9bdb-5179b1c41e16' AND dedup_key = 'co:nvidia|software-engineering-dynamo|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:de0502bc-ed79-4938-a1b6-31814f85dea1' WHERE id = 'de0502bc-ed79-4938-a1b6-31814f85dea1' AND dedup_key = 'co:ge vernova|energy-optimization-software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3860de9a-f93f-4437-8011-cc2deff05397' WHERE id = '3860de9a-f93f-4437-8011-cc2deff05397' AND dedup_key = 'co:ge vernova|project-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c5be20d3-2ca8-4c2c-87a3-95807d7875f2' WHERE id = 'c5be20d3-2ca8-4c2c-87a3-95807d7875f2' AND dedup_key = 'co:rtx|digital-support-business-intelligence-computer-science|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a751a17b-164f-46ca-b766-b877f51326a0' WHERE id = 'a751a17b-164f-46ca-b766-b877f51326a0' AND dedup_key = 'co:frost|computer-science-digital-services|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6bcd985c-83dd-463e-af07-1335190f5a6b' WHERE id = '6bcd985c-83dd-463e-af07-1335190f5a6b' AND dedup_key = 'co:general motors|lighting-software-development-test-co-op|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:8cdd0f44-0699-45ce-b250-d0b4b335b674' WHERE id = '8cdd0f44-0699-45ce-b250-d0b4b335b674' AND dedup_key = 'co:brunswick|software-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:4c6b86af-d300-44dc-9520-ff588cf77cb6' WHERE id = '4c6b86af-d300-44dc-9520-ff588cf77cb6' AND dedup_key = 'co:availity|software-engineer-multiple-teams|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ccc3d602-edbb-471f-a5a7-31c86b8e1be1' WHERE id = 'ccc3d602-edbb-471f-a5a7-31c86b8e1be1' AND dedup_key = 'co:elevate semiconductor|product-engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:44c6d0b5-3e6b-4934-a565-9263aa68ccd0' WHERE id = '44c6d0b5-3e6b-4934-a565-9263aa68ccd0' AND dedup_key = 'co:teledyne technologies|software-engineer|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:da7a8022-662b-45de-ad38-ca9cf9c8be6c' WHERE id = 'da7a8022-662b-45de-ad38-ca9cf9c8be6c' AND dedup_key = 'co:copart|devops-engineering|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:980e64a8-3d2c-40c8-934b-9f62ba90e7a3' WHERE id = '980e64a8-3d2c-40c8-934b-9f62ba90e7a3' AND dedup_key = 'co:copart|qa-engineering|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:460558ae-12f2-4ce7-a60c-bd8eb10879ee' WHERE id = '460558ae-12f2-4ce7-a60c-bd8eb10879ee' AND dedup_key = 'co:humana|software-engineer-centerwell-and-humana-military|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9001e0a5-7256-4b71-8187-e4d435998fa5' WHERE id = '9001e0a5-7256-4b71-8187-e4d435998fa5' AND dedup_key = 'co:raytheon|software-development|summer-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:939e2c58-ac83-4853-b09b-f31a420a7ce9' WHERE id = '939e2c58-ac83-4853-b09b-f31a420a7ce9' AND dedup_key = 'co:campbell soup|agentic-ai-engineer-co-op|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:8f2d3b53-ea18-4658-acd9-5516294b20c6' WHERE id = '8f2d3b53-ea18-4658-acd9-5516294b20c6' AND dedup_key = 'co:campbell soup|data-engineer-da-ai-co-op|fall-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6d71e617-960e-457b-b445-04ecc35f427e' WHERE id = '6d71e617-960e-457b-b445-04ecc35f427e' AND dedup_key = 'co:analog devices|systems-integration-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ac409ae3-6bff-4283-b00c-ebb50998fd2e' WHERE id = 'ac409ae3-6bff-4283-b00c-ebb50998fd2e' AND dedup_key = 'co:analog devices|algorithm-development-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:67c73d70-1223-492d-8ed6-1c8911e33a3b' WHERE id = '67c73d70-1223-492d-8ed6-1c8911e33a3b' AND dedup_key = 'co:hitachi|engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3b202a9e-30ed-47f5-b661-6616c23bccde' WHERE id = '3b202a9e-30ed-47f5-b661-6616c23bccde' AND dedup_key = 'co:bp|reservoir-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2e9e7357-08e1-44c3-bdb0-94c4168762be' WHERE id = '2e9e7357-08e1-44c3-bdb0-94c4168762be' AND dedup_key = 'co:salesforce|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7b80a14e-d35b-48c7-8c9e-fa822bba9647' WHERE id = '7b80a14e-d35b-48c7-8c9e-fa822bba9647' AND dedup_key = 'co:national laboratory of the rockies|software-and-data-infrastructure|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7a174162-718f-430a-86b4-84ec0bf041ed' WHERE id = '7a174162-718f-430a-86b4-84ec0bf041ed' AND dedup_key = 'co:ccc intelligent solutions|applied-ai-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3587f771-5997-4b00-8fe7-951450dd675a' WHERE id = '3587f771-5997-4b00-8fe7-951450dd675a' AND dedup_key = 'co:bank of montreal|full-stack-engineer-data-cognition-team|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a7b2ac78-0f7c-4fee-b740-6f4efda22cd6' WHERE id = 'a7b2ac78-0f7c-4fee-b740-6f4efda22cd6' AND dedup_key = 'co:booz allen|ai-ran-telecommunications-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3687f7f6-8224-4a71-890f-de4bff8e67d7' WHERE id = '3687f7f6-8224-4a71-890f-de4bff8e67d7' AND dedup_key = 'co:mastercard|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:49a71926-8218-4e74-b0ab-e46fe3dd4c6c' WHERE id = '49a71926-8218-4e74-b0ab-e46fe3dd4c6c' AND dedup_key = 'co:analog devices|digital-design-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2adbf822-bcfa-42fc-af26-4a0cbd96c3f5' WHERE id = '2adbf822-bcfa-42fc-af26-4a0cbd96c3f5' AND dedup_key = 'co:the walt disney|computer-science-computer-engineering-multiple-teams|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:38770edf-5bcc-4a4e-b83b-72a03f2cb1dc' WHERE id = '38770edf-5bcc-4a4e-b83b-72a03f2cb1dc' AND dedup_key = 'co:analog devices|product-engineer-product-development|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e0807f40-5698-482e-9be9-35349eced260' WHERE id = 'e0807f40-5698-482e-9be9-35349eced260' AND dedup_key = 'co:royal bank of canada|technology-developer-co-op-wealth-management|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:47f834f8-8fbb-42bc-baf3-9df0cd5d9937' WHERE id = '47f834f8-8fbb-42bc-baf3-9df0cd5d9937' AND dedup_key = 'co:motorola|android-application-developer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6eb2d2f9-8041-4ccd-9db9-6047c967cabc' WHERE id = '6eb2d2f9-8041-4ccd-9db9-6047c967cabc' AND dedup_key = 'co:freddie mac|software-developer-single-family|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:71ac5350-ba2f-479b-acb5-fd38593fd7e0' WHERE id = '71ac5350-ba2f-479b-acb5-fd38593fd7e0' AND dedup_key = 'co:freddie mac|multifamily-software-development|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2df4a1da-c3e9-419d-a58f-d19ed2ff8a3f' WHERE id = '2df4a1da-c3e9-419d-a58f-d19ed2ff8a3f' AND dedup_key = 'co:procter gamble|data-ai-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3356814a-8c4b-4c4a-81d2-2845002451dc' WHERE id = '3356814a-8c4b-4c4a-81d2-2845002451dc' AND dedup_key = 'co:ducharme mcmillen associates|software-developer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:23d72ff2-a484-4806-b407-78d58a74c68a' WHERE id = '23d72ff2-a484-4806-b407-78d58a74c68a' AND dedup_key = 'co:ducharme mcmillen associates|software-development|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:67ca9bcc-6257-4834-a19a-499f95e3b134' WHERE id = '67ca9bcc-6257-4834-a19a-499f95e3b134' AND dedup_key = 'co:micron technology|thin-films-equipment-engineering-ede|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a6a256f6-2edd-4d12-9744-7f2664ce4598' WHERE id = 'a6a256f6-2edd-4d12-9744-7f2664ce4598' AND dedup_key = 'co:monolithic power systems|ai-developer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:68a42408-ed6b-4628-a155-7836f1f50b04' WHERE id = '68a42408-ed6b-4628-a155-7836f1f50b04' AND dedup_key = 'co:the hartford|software-engineer-tech-data|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:966a6049-bc05-4757-9999-cdde4d9e16e5' WHERE id = '966a6049-bc05-4757-9999-cdde4d9e16e5' AND dedup_key = 'co:the hartford|data-engineer-technology-data-ai-and-operations|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9791bc0f-443f-4710-b25e-d93f57216b33' WHERE id = '9791bc0f-443f-4710-b25e-d93f57216b33' AND dedup_key = 'co:the hartford|data-engineer-tech-data|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:36cd7194-56cb-4ce9-bb42-a780b4c78dd8' WHERE id = '36cd7194-56cb-4ce9-bb42-a780b4c78dd8' AND dedup_key = 'co:brunswick|computer-graphics-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a635eb48-3a79-4864-a23b-2f18911f1741' WHERE id = 'a635eb48-3a79-4864-a23b-2f18911f1741' AND dedup_key = 'co:brunswick|software-engineer-boating-intelligence-design-lab|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:1d74cc80-172e-4223-8836-44dad15ef74e' WHERE id = '1d74cc80-172e-4223-8836-44dad15ef74e' AND dedup_key = 'co:bp|corporate-asset-development-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:1da691f8-d53b-49fe-bf7e-031bf14a5145' WHERE id = '1da691f8-d53b-49fe-bf7e-031bf14a5145' AND dedup_key = 'co:genentech|machine-learning-opregen-machine-learning|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:863c2c7c-1c7e-4d02-bc00-44bc176ed8d7' WHERE id = '863c2c7c-1c7e-4d02-bc00-44bc176ed8d7' AND dedup_key = 'co:wex|artificial-intelligence-ai-ml-nlp-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7caa91ea-b9a6-4760-b53d-47f5cd4aec75' WHERE id = '7caa91ea-b9a6-4760-b53d-47f5cd4aec75' AND dedup_key = 'co:the hartford|software-engineer-technology-data|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:23213059-9c79-48bb-9428-8652816ec09e' WHERE id = '23213059-9c79-48bb-9428-8652816ec09e' AND dedup_key = 'co:ge appliances|software-engineer-co-op|fall-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2ab2b88d-f5da-4f7c-92ae-1f45951d211a' WHERE id = '2ab2b88d-f5da-4f7c-92ae-1f45951d211a' AND dedup_key = 'co:ge appliances|engineering-co-op|fall-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:32f8d576-6f4c-4e80-b6aa-601c9196bb7d' WHERE id = '32f8d576-6f4c-4e80-b6aa-601c9196bb7d' AND dedup_key = 'co:brunswick|systems-engineer-co-op-software-engineering|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a3080d49-b4ed-4fb8-93ee-69e1dbb1bf3f' WHERE id = 'a3080d49-b4ed-4fb8-93ee-69e1dbb1bf3f' AND dedup_key = 'co:repsol|reservoir-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7bc530ed-7bcd-4c5b-a4d2-fbb24cea880c' WHERE id = '7bc530ed-7bcd-4c5b-a4d2-fbb24cea880c' AND dedup_key = 'co:bank of montreal|full-stack-engineer-co-op-data-cognition-team|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ba76e785-5847-4294-b060-c07aed97117e' WHERE id = 'ba76e785-5847-4294-b060-c07aed97117e' AND dedup_key = 'co:parsons|software|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a39064ae-c0a3-49be-904b-9f0913e8a079' WHERE id = 'a39064ae-c0a3-49be-904b-9f0913e8a079' AND dedup_key = 'co:auto owners insurance|data-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:5d8b9b3a-38a5-44fe-a800-a0a96f90d4ac' WHERE id = '5d8b9b3a-38a5-44fe-a800-a0a96f90d4ac' AND dedup_key = 'co:auto owners insurance|analytics-web-systems-developer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7c985043-03df-49cf-ab64-68372035049a' WHERE id = '7c985043-03df-49cf-ab64-68372035049a' AND dedup_key = 'co:the hartford|software-engineer-technology-data-ai-and-operations|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3feade02-fb9a-421b-bb96-f5e7bcabdd74' WHERE id = '3feade02-fb9a-421b-bb96-f5e7bcabdd74' AND dedup_key = 'co:ancestry|software-engineer-co-op-observability|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6efe828b-faa5-43f6-9cf1-a13cc5fe88f4' WHERE id = '6efe828b-faa5-43f6-9cf1-a13cc5fe88f4' AND dedup_key = 'co:caci|software-developer-data-scientist|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:37ce247c-4dab-4422-8592-8ad401305ecd' WHERE id = '37ce247c-4dab-4422-8592-8ad401305ecd' AND dedup_key = 'co:rtx|automation-solutions-developer|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e84176ed-1417-43fa-a11b-bd00221afd7a' WHERE id = 'e84176ed-1417-43fa-a11b-bd00221afd7a' AND dedup_key = 'co:leidos|software-engineer-artificial-intelligence|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:47fe3df5-c4be-465f-93e9-60eb9352d4d9' WHERE id = '47fe3df5-c4be-465f-93e9-60eb9352d4d9' AND dedup_key = 'co:repsol|production-allocation-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a324f366-46da-46f5-9ff9-ccd5ad464b93' WHERE id = 'a324f366-46da-46f5-9ff9-ccd5ad464b93' AND dedup_key = 'co:repsol|development-planning-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ce601ae2-3489-445c-958d-79d9ece21c60' WHERE id = 'ce601ae2-3489-445c-958d-79d9ece21c60' AND dedup_key = 'co:johnson johnson|software-engineer-co-op-medtech|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:1a5b4a87-3e34-43c3-932d-4664b5295584' WHERE id = '1a5b4a87-3e34-43c3-932d-4664b5295584' AND dedup_key = 'co:auto owners insurance|software-developer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:fc692b97-6e7d-43ac-8b86-bd74685c8901' WHERE id = 'fc692b97-6e7d-43ac-8b86-bd74685c8901' AND dedup_key = 'co:procter gamble|research-development-scientist-engineer-freshmen-sophomores-and-juniors|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a23c4d72-874d-4ee9-b3c4-36fdb53efac5' WHERE id = 'a23c4d72-874d-4ee9-b3c4-36fdb53efac5' AND dedup_key = 'co:dimensional fund advisors|investment-engineering-undergraduate-master-s|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2ede2e43-d02f-4a36-9e87-f563e13f99f6' WHERE id = '2ede2e43-d02f-4a36-9e87-f563e13f99f6' AND dedup_key = 'co:ambarella|dft-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a26b86e0-0e0a-4cbe-9339-063c90472988' WHERE id = 'a26b86e0-0e0a-4cbe-9339-063c90472988' AND dedup_key = 'co:ambarella|software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:0230fc8b-327e-4f06-8ebe-d39bc486e75d' WHERE id = '0230fc8b-327e-4f06-8ebe-d39bc486e75d' AND dedup_key = 'co:ambarella|algorithm-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e2559a75-d614-4170-a80b-366b38e0083a' WHERE id = 'e2559a75-d614-4170-a80b-366b38e0083a' AND dedup_key = 'co:ambarella|software-architecture|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c9037d61-5931-440e-9f1f-ed31deaa84bb' WHERE id = 'c9037d61-5931-440e-9f1f-ed31deaa84bb' AND dedup_key = 'co:booz allen|software-developer-university|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6a70d810-8b81-45d4-aa5b-4642ab420ade' WHERE id = '6a70d810-8b81-45d4-aa5b-4642ab420ade' AND dedup_key = 'co:booz allen|software-developer-games|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:23644019-1e80-4a25-b864-c6437fe07924' WHERE id = '23644019-1e80-4a25-b864-c6437fe07924' AND dedup_key = 'co:leidos|computer-engineering-co-op|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:390a0c19-3b06-4107-97d1-6ed7bbe5b826' WHERE id = '390a0c19-3b06-4107-97d1-6ed7bbe5b826' AND dedup_key = 'co:leidos|software-developer-co-op|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:08c41772-0de1-4d3b-a468-a1336f0e6f50' WHERE id = '08c41772-0de1-4d3b-a468-a1336f0e6f50' AND dedup_key = 'co:manulife financial|software-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ff06c66d-92f9-4c1c-a2f7-d2efe238b872' WHERE id = 'ff06c66d-92f9-4c1c-a2f7-d2efe238b872' AND dedup_key = 'co:microchip technology|engineering-applications|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:0b2db226-9db9-4aad-b4d2-93f661a6ca6f' WHERE id = '0b2db226-9db9-4aad-b4d2-93f661a6ca6f' AND dedup_key = 'co:medtronic|engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:0606e4aa-90e1-4ba4-9873-1fb9547b2ab5' WHERE id = '0606e4aa-90e1-4ba4-9873-1fb9547b2ab5' AND dedup_key = 'co:finastra|ai-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7f6b4775-dd78-4e51-9fcc-8397753ccbd6' WHERE id = '7f6b4775-dd78-4e51-9fcc-8397753ccbd6' AND dedup_key = 'co:royal bank of canada|data-analyst-developer-group-risk-management|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ba6d2323-8f60-4198-bfe5-568a761cc052' WHERE id = 'ba6d2323-8f60-4198-bfe5-568a761cc052' AND dedup_key = 'co:royal bank of canada|data-analyst-developer-8-months-group-risk-management|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c7efcf06-fae2-4ff7-873b-3a029a5db957' WHERE id = 'c7efcf06-fae2-4ff7-873b-3a029a5db957' AND dedup_key = 'co:leidos|data-engineering-analytics|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:26464b7b-e542-486c-8d00-5fd59274b795' WHERE id = '26464b7b-e542-486c-8d00-5fd59274b795' AND dedup_key = 'co:brunswick|engineering-validation|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:cea1a3a5-c2a1-44bb-b0cf-a86f42c0d978' WHERE id = 'cea1a3a5-c2a1-44bb-b0cf-a86f42c0d978' AND dedup_key = 'co:analog devices|ai-ml-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c0178f45-4188-40d0-9aae-3348239e31cd' WHERE id = 'c0178f45-4188-40d0-9aae-3348239e31cd' AND dedup_key = 'co:motorola|mission-critical-networks-software-engineer-co-op|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:311c3e25-9ea9-4584-a06b-ebc6e482ec23' WHERE id = '311c3e25-9ea9-4584-a06b-ebc6e482ec23' AND dedup_key = 'co:mastercard|data-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:59ca5337-2595-4774-b2b4-cfaec3b8bfab' WHERE id = '59ca5337-2595-4774-b2b4-cfaec3b8bfab' AND dedup_key = 'co:booz allen|ai-software-developer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:65983bc1-74a9-46e0-925d-e2266c8ea4a6' WHERE id = '65983bc1-74a9-46e0-925d-e2266c8ea4a6' AND dedup_key = 'co:booz allen|software-developer-university-games|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:48481e0d-31e2-49bf-9a15-5cb4dd540da4' WHERE id = '48481e0d-31e2-49bf-9a15-5cb4dd540da4' AND dedup_key = 'co:the walt disney|software-engineer|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6aa58a11-ff84-49e9-853a-3313df8a50f7' WHERE id = '6aa58a11-ff84-49e9-853a-3313df8a50f7' AND dedup_key = 'co:brunswick|software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a90cfbaa-1d75-42ea-a011-54ee775208be' WHERE id = 'a90cfbaa-1d75-42ea-a011-54ee775208be' AND dedup_key = 'co:schweitzer engineering laboratories|engineering-protection-systems-forensics|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c8e47d6f-7514-4528-a467-62455f03cda6' WHERE id = 'c8e47d6f-7514-4528-a467-62455f03cda6' AND dedup_key = 'co:philips|data-ai-ml-engineer-image-guided-therapy-devices-software-r-d|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f338c11e-9944-4169-898f-f811a6b89771' WHERE id = 'f338c11e-9944-4169-898f-f811a6b89771' AND dedup_key = 'co:cadence design systems|post-silicon-validation-engineering-characterization-and-support|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:583e679e-b183-47b6-bac8-9c36a3100c35' WHERE id = '583e679e-b183-47b6-bac8-9c36a3100c35' AND dedup_key = 'co:equifax|technology-software-development-site-reliability-engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a7ce2834-e8de-463e-a075-256d5a3dacce' WHERE id = 'a7ce2834-e8de-463e-a075-256d5a3dacce' AND dedup_key = 'co:intelcom dragonfly|full-stack-developer-route-optimization|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:88404599-3905-40f8-849c-fd58c68633e2' WHERE id = '88404599-3905-40f8-849c-fd58c68633e2' AND dedup_key = 'co:intelcom dragonfly|front-end-developer-mobile-application|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f5074b11-9079-4181-a60e-e57e64db97ad' WHERE id = 'f5074b11-9079-4181-a60e-e57e64db97ad' AND dedup_key = 'co:intelcom dragonfly|business-intelligence-developer-bi|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:66ad12e8-81bc-4db2-9394-1c852c2bb6e1' WHERE id = '66ad12e8-81bc-4db2-9394-1c852c2bb6e1' AND dedup_key = 'co:intelcom dragonfly|data-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:98b154d4-7cff-4d1a-8052-3b1a43d5230a' WHERE id = '98b154d4-7cff-4d1a-8052-3b1a43d5230a' AND dedup_key = 'co:ge aerospace|applied-ai-engineer-co-op|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:56093405-e5ef-4a46-aeb9-a9078e4b3de6' WHERE id = '56093405-e5ef-4a46-aeb9-a9078e4b3de6' AND dedup_key = 'co:stryker|software-engineering-software-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:31397148-26b5-4889-aa7b-7dc01294af6b' WHERE id = '31397148-26b5-4889-aa7b-7dc01294af6b' AND dedup_key = 'co:stryker|software-engineering-multiple-teams|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:cc176323-4119-4148-a588-fdce1180fff8' WHERE id = 'cc176323-4119-4148-a588-fdce1180fff8' AND dedup_key = 'co:copart|ai-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ccec06a0-c7a8-4bf6-b7ad-4ab1ee91d2b3' WHERE id = 'ccec06a0-c7a8-4bf6-b7ad-4ab1ee91d2b3' AND dedup_key = 'co:the home depot|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2543dbcf-4d1f-4241-b730-09aca6e1fb40' WHERE id = '2543dbcf-4d1f-4241-b730-09aca6e1fb40' AND dedup_key = 'co:medline|rpa-agentic-ai-software-technologies|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d5f46fe1-4a73-44ea-8fb7-b99accddb887' WHERE id = 'd5f46fe1-4a73-44ea-8fb7-b99accddb887' AND dedup_key = 'co:draftkings|software-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:77626970-4241-4468-8226-9552bc9d2a19' WHERE id = '77626970-4241-4468-8226-9552bc9d2a19' AND dedup_key = 'co:draftkings|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:121ea21f-5b17-44e4-aa18-e424b748fea7' WHERE id = '121ea21f-5b17-44e4-aa18-e424b748fea7' AND dedup_key = 'co:nisource|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e3728655-5e92-4fdb-9b4b-ced32dbfbeb1' WHERE id = 'e3728655-5e92-4fdb-9b4b-ced32dbfbeb1' AND dedup_key = 'co:oshkosh|software-engineer-software|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:01f5f170-f8e9-4388-9ffb-70622fe4d329' WHERE id = '01f5f170-f8e9-4388-9ffb-70622fe4d329' AND dedup_key = 'co:cibc|software-engineer-co-op|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d0b4a134-2a08-4f04-98cb-455ff2d6a6b9' WHERE id = 'd0b4a134-2a08-4f04-98cb-455ff2d6a6b9' AND dedup_key = 'co:manulife financial|software-engineering-co-op|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:fb6736ea-1204-47a3-96e2-54eb846705e3' WHERE id = 'fb6736ea-1204-47a3-96e2-54eb846705e3' AND dedup_key = 'co:manulife financial|data-engineer-co-op-data-engineering|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:662d2eb3-1526-407c-bcc0-40bd2181dcaa' WHERE id = '662d2eb3-1526-407c-bcc0-40bd2181dcaa' AND dedup_key = 'co:manulife financial|software-engineer-software-engineering|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:45476c70-3868-4287-b1a5-089ceeeefaa2' WHERE id = '45476c70-3868-4287-b1a5-089ceeeefaa2' AND dedup_key = 'co:general motors|data-engineering-software-developer-co-op|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:23d5df93-9637-4c04-a2c4-aa73bbb76c6c' WHERE id = '23d5df93-9637-4c04-a2c4-aa73bbb76c6c' AND dedup_key = 'co:nike|machine-learning-engineering-undergraduate-artificial-intelligence-data|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c799d6e7-bbda-4c4a-b65b-88847e70243f' WHERE id = 'c799d6e7-bbda-4c4a-b65b-88847e70243f' AND dedup_key = 'co:ge aerospace|product-definition-engineer-designer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:025f5632-522a-4c2a-8600-9cc4895baa89' WHERE id = '025f5632-522a-4c2a-8600-9cc4895baa89' AND dedup_key = 'co:nike|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:19db709b-3f6e-497e-9c47-f0bde2c42405' WHERE id = '19db709b-3f6e-497e-9c47-f0bde2c42405' AND dedup_key = 'co:amentum|software-engineer-space-force-range-contract|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3d025d6e-9284-424b-b82e-7d2f64a9874b' WHERE id = '3d025d6e-9284-424b-b82e-7d2f64a9874b' AND dedup_key = 'co:caci|software-engineer-co-op|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:bab1dade-4df9-425f-ade5-e6c4179df220' WHERE id = 'bab1dade-4df9-425f-ade5-e6c4179df220' AND dedup_key = 'co:caci|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:b597d520-9b61-4cdd-8c6e-c07f8147259c' WHERE id = 'b597d520-9b61-4cdd-8c6e-c07f8147259c' AND dedup_key = 'co:caci|software-engineer-co-op|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2cf0f26e-090d-43bc-b123-533f38de0db1' WHERE id = '2cf0f26e-090d-43bc-b123-533f38de0db1' AND dedup_key = 'co:ge healthcare|engineering-development-software|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3f7fd285-4d1e-40a9-8bc8-09b107ee933e' WHERE id = '3f7fd285-4d1e-40a9-8bc8-09b107ee933e' AND dedup_key = 'co:micron technology|ai-systems-and-infrastructure-engineering|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:1efcf523-c326-4bbc-b131-ece3c213387f' WHERE id = '1efcf523-c326-4bbc-b131-ece3c213387f' AND dedup_key = 'co:draper|optics-physics-sensor-engineering-co-op|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d989c117-0da2-4e11-9400-3b346a18f9af' WHERE id = 'd989c117-0da2-4e11-9400-3b346a18f9af' AND dedup_key = 'co:igs energy|software-engineer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a6ed9765-78d4-4c7a-859a-5da3e0ea24da' WHERE id = 'a6ed9765-78d4-4c7a-859a-5da3e0ea24da' AND dedup_key = 'co:brunswick|software-controls-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:74f3b561-04c3-4b0d-82fd-1036a70ff453' WHERE id = '74f3b561-04c3-4b0d-82fd-1036a70ff453' AND dedup_key = 'co:philips|cybersecurity-data-analytics-co-op-ultrasound-regulatory-affairs|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:18cbde3f-b37b-4192-b643-e5dd9cc45ccc' WHERE id = '18cbde3f-b37b-4192-b643-e5dd9cc45ccc' AND dedup_key = 'co:adobe|machine-learning-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a1b1f44f-6b66-4910-a655-9c546934a260' WHERE id = 'a1b1f44f-6b66-4910-a655-9c546934a260' AND dedup_key = 'co:travelers|engineering-development|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:54315c65-6b92-420b-be9a-4b743ed61c0f' WHERE id = '54315c65-6b92-420b-be9a-4b743ed61c0f' AND dedup_key = 'co:tc energy|engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:44f3e4ca-586b-4d52-a86e-bf010b611201' WHERE id = '44f3e4ca-586b-4d52-a86e-bf010b611201' AND dedup_key = 'co:southwest airlines|software-engineer-multiple-teams|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d9028163-b2cf-48b7-9d6f-efccf8c10d51' WHERE id = 'd9028163-b2cf-48b7-9d6f-efccf8c10d51' AND dedup_key = 'co:fifth third bank|software-engineer-co-op-enterprise-finance-applications|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:e46962fa-af9f-41c1-86df-53943d926482' WHERE id = 'e46962fa-af9f-41c1-86df-53943d926482' AND dedup_key = 'co:tc energy|engineering-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9081172b-7f90-4952-acdb-9bf1f21f935e' WHERE id = '9081172b-7f90-4952-acdb-9bf1f21f935e' AND dedup_key = 'co:vermeer|embedded-software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3accbec6-ed81-4459-93f0-29c3ec261cb5' WHERE id = '3accbec6-ed81-4459-93f0-29c3ec261cb5' AND dedup_key = 'co:tc energy|engineering|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:df44b4a1-da53-4f7c-851e-3aa5a343ac4b' WHERE id = 'df44b4a1-da53-4f7c-851e-3aa5a343ac4b' AND dedup_key = 'co:rtx|numerical-methods-advanced-software-development|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2948e2a6-15fd-40f5-a331-51f16e95fbd6' WHERE id = '2948e2a6-15fd-40f5-a331-51f16e95fbd6' AND dedup_key = 'co:caci|embedded-software-engineer-co-op|fall-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a9c9d116-df26-4efb-8ecc-df2cb57e25de' WHERE id = 'a9c9d116-df26-4efb-8ecc-df2cb57e25de' AND dedup_key = 'co:caci|embedded-software-engineer-co-op|spring-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c02bacf0-7540-499b-92af-6b976f641154' WHERE id = 'c02bacf0-7540-499b-92af-6b976f641154' AND dedup_key = 'co:magna|software-engineering-co-op|any-any';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9814475d-a99e-4dea-8547-6963d1cc7287' WHERE id = '9814475d-a99e-4dea-8547-6963d1cc7287' AND dedup_key = 'co:nisource|engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:88c8c512-735d-46e3-82c2-d14a81d6bb17' WHERE id = '88c8c512-735d-46e3-82c2-d14a81d6bb17' AND dedup_key = 'co:pennsylvania state university|research-engineering-applied-research-laboratory|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:41adb144-1152-45f8-a05e-c763c77c2d83' WHERE id = '41adb144-1152-45f8-a05e-c763c77c2d83' AND dedup_key = 'co:vermeer|component-engineer|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ee9a7a32-6a72-40c3-9cd4-4bdcea465d1f' WHERE id = 'ee9a7a32-6a72-40c3-9cd4-4bdcea465d1f' AND dedup_key = 'co:booz allen|systems-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:84345318-a996-494b-aeb4-b60b5e4f8700' WHERE id = '84345318-a996-494b-aeb4-b60b5e4f8700' AND dedup_key = 'co:booz allen|systems-engineer-games|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f2e2ed4f-5e65-402d-b35d-f035473d3e2a' WHERE id = 'f2e2ed4f-5e65-402d-b35d-f035473d3e2a' AND dedup_key = 'co:booz allen|systems-engineer-university|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:df42698d-83a3-4d48-8874-f455131cbb77' WHERE id = 'df42698d-83a3-4d48-8874-f455131cbb77' AND dedup_key = 'co:trumpf|application-engineer-co-op|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:68c703f9-a8a1-4d98-913f-9119ac1563e5' WHERE id = '68c703f9-a8a1-4d98-913f-9119ac1563e5' AND dedup_key = 'co:stanley black decker|embedded-software-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2e92e618-7647-41f1-8549-f1f260f740c2' WHERE id = '2e92e618-7647-41f1-8549-f1f260f740c2' AND dedup_key = 'co:teledyne|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:20e68190-6587-479c-87c4-84258b308b91' WHERE id = '20e68190-6587-479c-87c4-84258b308b91' AND dedup_key = 'co:micron technology|systems-performance-engineer|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:26d037ae-b926-4f3b-9626-81c94acd3810' WHERE id = '26d037ae-b926-4f3b-9626-81c94acd3810' AND dedup_key = 'co:medline|software-development|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:cccfec40-58a9-4258-b6c0-2fe89a3080d5' WHERE id = 'cccfec40-58a9-4258-b6c0-2fe89a3080d5' AND dedup_key = 'co:blue origin|software-developer-avionics-software|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:306f4cca-6a8b-485f-bf68-accda485e779' WHERE id = '306f4cca-6a8b-485f-bf68-accda485e779' AND dedup_key = 'co:blue origin|avionics-software|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2f03f5ca-6755-42e6-abbc-3bee690f16fa' WHERE id = '2f03f5ca-6755-42e6-abbc-3bee690f16fa' AND dedup_key = 'co:blue origin|avionics-software-undergraduate|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:05af2008-3382-40a5-8614-444a71054727' WHERE id = '05af2008-3382-40a5-8614-444a71054727' AND dedup_key = 'co:blue origin|test-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:6221c6d7-597b-41e1-84b6-bbd45b977082' WHERE id = '6221c6d7-597b-41e1-84b6-bbd45b977082' AND dedup_key = 'co:blue origin|software-developer-undergraduate|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:1844d494-1060-4f24-9af2-644def72f9e5' WHERE id = '1844d494-1060-4f24-9af2-644def72f9e5' AND dedup_key = 'co:philips|software-engineer-co-op-r-d|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c04d5f4c-460c-4430-a563-a95bdb8a8b91' WHERE id = 'c04d5f4c-460c-4430-a563-a95bdb8a8b91' AND dedup_key = 'co:booz allen|software-developer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:038eb441-ccf4-455e-8f4f-d75b20b375a7' WHERE id = '038eb441-ccf4-455e-8f4f-d75b20b375a7' AND dedup_key = 'co:philips|design-release-engineer-co-op|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:60399bb1-d3fe-4f39-88e6-f532ec6dcfc7' WHERE id = '60399bb1-d3fe-4f39-88e6-f532ec6dcfc7' AND dedup_key = 'co:philips|data-engineering-co-op|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:288d2881-a72e-4ba5-950c-58b151eddf43' WHERE id = '288d2881-a72e-4ba5-950c-58b151eddf43' AND dedup_key = 'co:philips|software-engineering-co-op-apm|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:70779e40-d3aa-4ee9-8671-96b6bc3498a5' WHERE id = '70779e40-d3aa-4ee9-8671-96b6bc3498a5' AND dedup_key = 'co:philips|data-engineering-co-op|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:d57d9a15-0103-4de8-8b78-b39006708b0e' WHERE id = 'd57d9a15-0103-4de8-8b78-b39006708b0e' AND dedup_key = 'co:johnson johnson|software-engineer-co-op|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f0aefc69-5099-4155-93f3-1ec4fd173bf2' WHERE id = 'f0aefc69-5099-4155-93f3-1ec4fd173bf2' AND dedup_key = 'co:rtx|technical-publications-technical-developer-artificial-intelligence-machine-learning|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:dfd29b25-32f6-4ca6-8d39-92a9eb6e8bae' WHERE id = 'dfd29b25-32f6-4ca6-8d39-92a9eb6e8bae' AND dedup_key = 'co:intelcom dragonfly|software-developer-address-intelligence-platform|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9ab9318a-4450-4e85-8a1e-6a404d99baca' WHERE id = '9ab9318a-4450-4e85-8a1e-6a404d99baca' AND dedup_key = 'co:newrez|software-developer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:4ca35f82-dfe4-4668-a7b9-8a3cdfad6ecb' WHERE id = '4ca35f82-dfe4-4668-a7b9-8a3cdfad6ecb' AND dedup_key = 'co:clarios|forward-deployed-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ab3be926-a244-41e6-bd75-9ec56c018652' WHERE id = 'ab3be926-a244-41e6-bd75-9ec56c018652' AND dedup_key = 'co:geico|artificial-intelligence-applied-research-machine-learning-phd|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:5c1b34de-6647-4da1-b480-228b1afb91d4' WHERE id = '5c1b34de-6647-4da1-b480-228b1afb91d4' AND dedup_key = 'co:nike|global-apparel-materials-developer-apparel-development-global-apparel-materials|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ebbbff0e-417e-492b-8f9a-d6dfe368bff3' WHERE id = 'ebbbff0e-417e-492b-8f9a-d6dfe368bff3' AND dedup_key = 'co:clearwater analytics|salesforce-developer|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7f2a6f9d-fb7d-47e7-8e95-c7118ba9e41f' WHERE id = '7f2a6f9d-fb7d-47e7-8e95-c7118ba9e41f' AND dedup_key = 'co:twg global|ai-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ba6661c7-f32e-4967-931c-1f0bfacdfaaf' WHERE id = 'ba6661c7-f32e-4967-931c-1f0bfacdfaaf' AND dedup_key = 'co:onlogic|firmware-engineer-co-op|winter-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:832bdf3d-3de6-4cc1-b5a8-423f539fcbab' WHERE id = '832bdf3d-3de6-4cc1-b5a8-423f539fcbab' AND dedup_key = 'co:genuine parts|customer-software-development|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:58382d6f-b6e6-45c7-84a9-cd06c10c5c3a' WHERE id = '58382d6f-b6e6-45c7-84a9-cd06c10c5c3a' AND dedup_key = 'co:genuine parts|software-developer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:202edd56-98fd-4246-9fec-39e8e11e5e5a' WHERE id = '202edd56-98fd-4246-9fec-39e8e11e5e5a' AND dedup_key = 'co:mcgill university|web-development|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:40010250-44f4-40b0-aa71-8a26c6649cbb' WHERE id = '40010250-44f4-40b0-aa71-8a26c6649cbb' AND dedup_key = 'co:parsons|software-developer|fall-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a06b093e-cb4b-4a09-8c85-1f135b67e27c' WHERE id = 'a06b093e-cb4b-4a09-8c85-1f135b67e27c' AND dedup_key = 'co:medline|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:9d5719c5-df03-4650-b38e-a778333fb9d1' WHERE id = '9d5719c5-df03-4650-b38e-a778333fb9d1' AND dedup_key = 'co:us foods|software-engineer-digital-commerce|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c4b715dc-1da8-4461-854c-e74c7386b620' WHERE id = 'c4b715dc-1da8-4461-854c-e74c7386b620' AND dedup_key = 'co:intelcom dragonfly|front-end-developer-power-platform-integration|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f321a663-35f0-49a4-b559-2b2e7848acf4' WHERE id = 'f321a663-35f0-49a4-b559-2b2e7848acf4' AND dedup_key = 'co:us foods|software-engineer-legacy-systems|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f268456c-0cba-45f8-aa73-d83fbf3126ea' WHERE id = 'f268456c-0cba-45f8-aa73-d83fbf3126ea' AND dedup_key = 'co:poet|process-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c7301677-c865-490a-9f28-86c40971a00d' WHERE id = 'c7301677-c865-490a-9f28-86c40971a00d' AND dedup_key = 'co:us foods|data-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:99a33ef9-e855-49af-a119-483cc6c0a10e' WHERE id = '99a33ef9-e855-49af-a119-483cc6c0a10e' AND dedup_key = 'co:first national bank|ai-machine-learning-modeler|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:2c36b39f-0abc-4122-929a-6cfe210f3fa4' WHERE id = '2c36b39f-0abc-4122-929a-6cfe210f3fa4' AND dedup_key = 'co:first national bank|data-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:bd21a75c-d5c1-4db4-ad17-347badb0036d' WHERE id = 'bd21a75c-d5c1-4db4-ad17-347badb0036d' AND dedup_key = 'co:first national bank|data-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c85e296e-57c6-4ad3-886d-9e25013c6698' WHERE id = 'c85e296e-57c6-4ad3-886d-9e25013c6698' AND dedup_key = 'co:aerovironment|embedded-software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:3b84ee5d-1965-4aa9-8637-23dc22cba0b0' WHERE id = '3b84ee5d-1965-4aa9-8637-23dc22cba0b0' AND dedup_key = 'co:aerovironment|machine-learning|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:c83a38b3-5feb-4657-a5ac-22d710bf30d5' WHERE id = 'c83a38b3-5feb-4657-a5ac-22d710bf30d5' AND dedup_key = 'co:aerovironment|software-engineer|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:927bda0d-9e2d-492d-a33b-04c0730fa237' WHERE id = '927bda0d-9e2d-492d-a33b-04c0730fa237' AND dedup_key = 'co:aerovironment|software-engineer|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:240eebea-de04-4704-84d6-ce3ef14b2990' WHERE id = '240eebea-de04-4704-84d6-ce3ef14b2990' AND dedup_key = 'co:vermeer|software-engineer-it|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:bc30a8d9-f439-4a8c-aeaf-21127da962bd' WHERE id = 'bc30a8d9-f439-4a8c-aeaf-21127da962bd' AND dedup_key = 'co:rockwell automation|content-software-development-lifecycle-services|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:a055bf39-d4af-4632-909c-83d79b9fe497' WHERE id = 'a055bf39-d4af-4632-909c-83d79b9fe497' AND dedup_key = 'co:northern trust|technology-software-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7ce64850-5a29-48ca-adac-b8f614e0ef2b' WHERE id = '7ce64850-5a29-48ca-adac-b8f614e0ef2b' AND dedup_key = 'co:clearwater analytics|quant-developer|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:dcd42dfa-c0f1-435e-a24e-75a25b8e6880' WHERE id = 'dcd42dfa-c0f1-435e-a24e-75a25b8e6880' AND dedup_key = 'co:clearwater analytics|software-development|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:7e6ad594-96d0-40a2-8355-eba62086a3e5' WHERE id = '7e6ad594-96d0-40a2-8355-eba62086a3e5' AND dedup_key = 'co:clearwater analytics|software-engineer-technical-product-management|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:f54a2c64-cbf9-42b6-9e4d-c9cf1d6d08f5' WHERE id = 'f54a2c64-cbf9-42b6-9e4d-c9cf1d6d08f5' AND dedup_key = 'co:michelin|data-engineering|summer-2027';
UPDATE internship_postings SET dedup_key = 'rekey-0025:4721be6e-7e99-4fd2-820d-237c8192d5fb' WHERE id = '4721be6e-7e99-4fd2-820d-237c8192d5fb' AND dedup_key = 'co:rtx|test-systems-engineer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:8ced0d9e-a6e8-421a-bc0f-50158debc94d' WHERE id = '8ced0d9e-a6e8-421a-bc0f-50158debc94d' AND dedup_key = 'co:genuine parts|web-developer|summer-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:ee4f664e-4994-4c3c-a688-7ebcd72ffe1b' WHERE id = 'ee4f664e-4994-4c3c-a688-7ebcd72ffe1b' AND dedup_key = 'co:genuine parts|software-engineer-qa-analyst|winter-2026';
UPDATE internship_postings SET dedup_key = 'rekey-0025:37ca8fd0-05c0-43de-8c2f-f34a5462b955' WHERE id = '37ca8fd0-05c0-43de-8c2f-f34a5462b955' AND dedup_key = 'co:genuine parts|software-engineer-quality-assurance-analyst|winter-2026';

-- 3. Claim the new keys.
UPDATE internship_postings SET dedup_key = 'ats:rippling:spreeai:c52472cb-2671-45d7-b666-17196dc3df25' WHERE id = '7ffa0ab3-c04a-474c-b47f-83ac85aea107' AND dedup_key = 'rekey-0025:7ffa0ab3-c04a-474c-b47f-83ac85aea107';
UPDATE internship_postings SET dedup_key = 'ats:rippling:spreeai:d34aed29-7a11-4e37-b5bc-e9317f82f0b1' WHERE id = '007c17f2-8453-470e-bc76-048b7732491c' AND dedup_key = 'rekey-0025:007c17f2-8453-470e-bc76-048b7732491c';
UPDATE internship_postings SET dedup_key = 'ats:workable:pony-dot-ai:BA5FFDBC71' WHERE id = '335af14f-07e8-40d5-9920-9545fbfbbeb8' AND dedup_key = 'rekey-0025:335af14f-07e8-40d5-9920-9545fbfbbeb8';
UPDATE internship_postings SET dedup_key = 'ats:workday:amgen.wd1:R-249424' WHERE id = '73165525-ca05-438c-95c3-b3304d955d37' AND dedup_key = 'rekey-0025:73165525-ca05-438c-95c3-b3304d955d37';
UPDATE internship_postings SET dedup_key = 'ats:workday:capitalone.wd12:R249013' WHERE id = 'bdeacc5f-bd00-4133-907c-5d4e36d8b827' AND dedup_key = 'rekey-0025:bdeacc5f-bd00-4133-907c-5d4e36d8b827';
UPDATE internship_postings SET dedup_key = 'ats:workday:capitalone.wd12:R249015' WHERE id = 'ccabdb42-a939-4c6c-acb5-2ab4db5ee463' AND dedup_key = 'rekey-0025:ccabdb42-a939-4c6c-acb5-2ab4db5ee463';
UPDATE internship_postings SET dedup_key = 'ats:workday:capitalone.wd12:R249022' WHERE id = '676b232b-10c4-4521-91b0-196e664bff66' AND dedup_key = 'rekey-0025:676b232b-10c4-4521-91b0-196e664bff66';
UPDATE internship_postings SET dedup_key = 'ats:workday:coreandmain.wd1:45804' WHERE id = '31aa88e6-d45a-4c7b-93c0-e041266f03ca' AND dedup_key = 'rekey-0025:31aa88e6-d45a-4c7b-93c0-e041266f03ca';
UPDATE internship_postings SET dedup_key = 'ats:workday:crowe.wd12:R-71041' WHERE id = '41646c4c-f8b9-4149-bacf-e29782c666a4' AND dedup_key = 'rekey-0025:41646c4c-f8b9-4149-bacf-e29782c666a4';
UPDATE internship_postings SET dedup_key = 'ats:workday:leidos.wd5:R-00189691' WHERE id = '7c3ef191-e361-486d-a0e8-2a98c14e5a3c' AND dedup_key = 'rekey-0025:7c3ef191-e361-486d-a0e8-2a98c14e5a3c';
UPDATE internship_postings SET dedup_key = 'ats:workday:monolithicpower.wd12:R-890' WHERE id = '3ea00598-eebf-432f-97e5-e701b54ea854' AND dedup_key = 'rekey-0025:3ea00598-eebf-432f-97e5-e701b54ea854';
UPDATE internship_postings SET dedup_key = 'ats:workday:osv-cci.wd1:R1346' WHERE id = 'b1c098a2-d9a4-418a-ae0d-d6fb58d2cdf7' AND dedup_key = 'rekey-0025:b1c098a2-d9a4-418a-ae0d-d6fb58d2cdf7';
UPDATE internship_postings SET dedup_key = 'ats:workday:osv-cci.wd1:R1350' WHERE id = 'd8736927-c88b-4ed9-9400-d5fafae4df0e' AND dedup_key = 'rekey-0025:d8736927-c88b-4ed9-9400-d5fafae4df0e';
UPDATE internship_postings SET dedup_key = 'ats:workday:avav.wd1:6367' WHERE id = '62ca9c5c-5da2-4f74-9620-0585be3e83e0' AND dedup_key = 'rekey-0025:62ca9c5c-5da2-4f74-9620-0585be3e83e0';
UPDATE internship_postings SET dedup_key = 'ats:workday:marvell.wd1:2502662' WHERE id = 'f52f4dce-dfd7-4d2a-8738-104ef5ac1fde' AND dedup_key = 'rekey-0025:f52f4dce-dfd7-4d2a-8738-104ef5ac1fde';
UPDATE internship_postings SET dedup_key = 'ats:workday:haier.wd3:REQ-24832' WHERE id = '6cb1657d-47cd-4feb-9360-04442f366608' AND dedup_key = 'rekey-0025:6cb1657d-47cd-4feb-9360-04442f366608';
UPDATE internship_postings SET dedup_key = 'ats:workday:aptiv.wd5:J000691719' WHERE id = '02dc0d35-a80e-46d2-b1be-212c16aa13c2' AND dedup_key = 'rekey-0025:02dc0d35-a80e-46d2-b1be-212c16aa13c2';
UPDATE internship_postings SET dedup_key = 'ats:workday:expedia.wd108:R-98585' WHERE id = '337331d5-276e-4459-9d80-b71b2e780e25' AND dedup_key = 'rekey-0025:337331d5-276e-4459-9d80-b71b2e780e25';
UPDATE internship_postings SET dedup_key = 'ats:workday:cadence.wd1:R53282-1' WHERE id = '3443738d-ff70-4a96-8a86-149c1ebea8c5' AND dedup_key = 'rekey-0025:3443738d-ff70-4a96-8a86-149c1ebea8c5';
UPDATE internship_postings SET dedup_key = 'ats:workday:psu.wd1:REQ_0000076214-1' WHERE id = 'b9315d46-c801-48df-8279-f771eb22c1ea' AND dedup_key = 'rekey-0025:b9315d46-c801-48df-8279-f771eb22c1ea';
UPDATE internship_postings SET dedup_key = 'ats:workday:marmon.wd501:JR0000037453-1' WHERE id = '2323b70f-37d6-44cf-a8e3-89f8154fcc72' AND dedup_key = 'rekey-0025:2323b70f-37d6-44cf-a8e3-89f8154fcc72';
UPDATE internship_postings SET dedup_key = 'ats:workday:marmon.wd501:JR0000037451' WHERE id = '651ac0b4-8210-453e-91ba-9be5bf5c354d' AND dedup_key = 'rekey-0025:651ac0b4-8210-453e-91ba-9be5bf5c354d';
UPDATE internship_postings SET dedup_key = 'ats:workday:oxy.wd5:JR100413' WHERE id = '617f1110-515f-4986-a652-dea958c7aba1' AND dedup_key = 'rekey-0025:617f1110-515f-4986-a652-dea958c7aba1';
UPDATE internship_postings SET dedup_key = 'ats:workday:nwis.wd12:JR101095' WHERE id = 'edd2fdf3-9c23-4f3f-8ca0-b8714980bd0b' AND dedup_key = 'rekey-0025:edd2fdf3-9c23-4f3f-8ca0-b8714980bd0b';
UPDATE internship_postings SET dedup_key = 'ats:workday:hp.wd5:3160410-1' WHERE id = '38b835f8-2d5f-4e83-8d0b-56db76441e42' AND dedup_key = 'rekey-0025:38b835f8-2d5f-4e83-8d0b-56db76441e42';
UPDATE internship_postings SET dedup_key = 'ats:workday:arlo.wd12:JR100299' WHERE id = 'ff57b238-d736-4763-a3f2-40ce59609b95' AND dedup_key = 'rekey-0025:ff57b238-d736-4763-a3f2-40ce59609b95';
UPDATE internship_postings SET dedup_key = 'ats:workday:arlo.wd12:JR100300' WHERE id = '6b8f6d3d-ed92-4f8a-a5d2-c36042364ba7' AND dedup_key = 'rekey-0025:6b8f6d3d-ed92-4f8a-a5d2-c36042364ba7';
UPDATE internship_postings SET dedup_key = 'ats:workday:amat.wd1:R2616095' WHERE id = '6a076fe6-b50e-4f4b-aebc-a2fd28251c61' AND dedup_key = 'rekey-0025:6a076fe6-b50e-4f4b-aebc-a2fd28251c61';
UPDATE internship_postings SET dedup_key = 'ats:workday:nio.wd3:R-000119' WHERE id = '503bd406-9309-4bce-a26b-0204619660e2' AND dedup_key = 'rekey-0025:503bd406-9309-4bce-a26b-0204619660e2';
UPDATE internship_postings SET dedup_key = 'ats:workday:thermofisher.wd5:R-01329364' WHERE id = '2e19ed93-bd38-4057-aebe-22d3c8de7e6b' AND dedup_key = 'rekey-0025:2e19ed93-bd38-4057-aebe-22d3c8de7e6b';
UPDATE internship_postings SET dedup_key = 'ats:rippling:omnis-corporation:e389ff2d-5be5-4571-8cc1-f361a139b753' WHERE id = '394656c8-ba01-4e42-95be-4cdbfc43f364' AND dedup_key = 'rekey-0025:394656c8-ba01-4e42-95be-4cdbfc43f364';
UPDATE internship_postings SET dedup_key = 'ats:workday:tencent.wd1:R107162' WHERE id = '40f820fb-278f-4196-9f69-a8772adde15b' AND dedup_key = 'rekey-0025:40f820fb-278f-4196-9f69-a8772adde15b';
UPDATE internship_postings SET dedup_key = 'ats:workday:corpay.wd103:R05866' WHERE id = '76fb57ed-a9e7-43df-a35c-c3d34366207e' AND dedup_key = 'rekey-0025:76fb57ed-a9e7-43df-a35c-c3d34366207e';
UPDATE internship_postings SET dedup_key = 'ats:workday:intel.wd1:JR0282639' WHERE id = '2a7245ca-271c-4afa-a55a-5966abd2da46' AND dedup_key = 'rekey-0025:2a7245ca-271c-4afa-a55a-5966abd2da46';
UPDATE internship_postings SET dedup_key = 'ats:workable:tmeic-corporation-americas:FD4C9770FF' WHERE id = '4960ecb1-b6ab-42c5-b593-73963f8888de' AND dedup_key = 'rekey-0025:4960ecb1-b6ab-42c5-b593-73963f8888de';
UPDATE internship_postings SET dedup_key = 'ats:workday:kla.wd1:2531653' WHERE id = 'b9759705-5f64-4d84-aa73-707053cf053c' AND dedup_key = 'rekey-0025:b9759705-5f64-4d84-aa73-707053cf053c';
UPDATE internship_postings SET dedup_key = 'ats:workable:eluvio:F70F3473E7' WHERE id = '39d0be1a-e745-4bf2-bf96-5f3396bb1d2c' AND dedup_key = 'rekey-0025:39d0be1a-e745-4bf2-bf96-5f3396bb1d2c';
UPDATE internship_postings SET dedup_key = 'ats:workday:cisive.wd108:JR100290' WHERE id = 'd7f2cafb-488e-4bc9-a5eb-979a8435a1d8' AND dedup_key = 'rekey-0025:d7f2cafb-488e-4bc9-a5eb-979a8435a1d8';
UPDATE internship_postings SET dedup_key = 'ats:workday:cae.wd3:118040' WHERE id = 'fc556629-5774-4ee2-b580-20bebae33251' AND dedup_key = 'rekey-0025:fc556629-5774-4ee2-b580-20bebae33251';
UPDATE internship_postings SET dedup_key = 'ats:workday:firstquality.wd5:R9813' WHERE id = '296394cc-8c29-42a5-bc0a-f34d24a4bd97' AND dedup_key = 'rekey-0025:296394cc-8c29-42a5-bc0a-f34d24a4bd97';
UPDATE internship_postings SET dedup_key = 'ats:workday:jadeglobal.wd5:R-103685' WHERE id = 'b4dc3766-66d3-421d-81de-69f6c4727e6a' AND dedup_key = 'rekey-0025:b4dc3766-66d3-421d-81de-69f6c4727e6a';
UPDATE internship_postings SET dedup_key = 'ats:workday:menasha.wd12:R13999' WHERE id = 'e1e021e2-ea71-4e88-9c7a-13b2435abc2c' AND dedup_key = 'rekey-0025:e1e021e2-ea71-4e88-9c7a-13b2435abc2c';
UPDATE internship_postings SET dedup_key = 'ats:workday:magna.wd3:R00243272' WHERE id = '781b3382-f1b0-4b6d-8dc9-eef84361aac7' AND dedup_key = 'rekey-0025:781b3382-f1b0-4b6d-8dc9-eef84361aac7';
UPDATE internship_postings SET dedup_key = 'ats:workday:marvell.wd1:2502662-1' WHERE id = '8797a520-c360-49e0-9c72-f6d08e92011b' AND dedup_key = 'rekey-0025:8797a520-c360-49e0-9c72-f6d08e92011b';
UPDATE internship_postings SET dedup_key = 'ats:workday:magna.wd3:R00244793' WHERE id = '1e6833c7-ca7b-4729-9f0c-79fefa404109' AND dedup_key = 'rekey-0025:1e6833c7-ca7b-4729-9f0c-79fefa404109';
UPDATE internship_postings SET dedup_key = 'ats:workable:altom-transport:9FC654F05E' WHERE id = 'be01b8c4-57bd-4ac4-8dab-c0a710365ca9' AND dedup_key = 'rekey-0025:be01b8c4-57bd-4ac4-8dab-c0a710365ca9';
UPDATE internship_postings SET dedup_key = 'ats:rippling:rippling:82c13e8f-ae96-4c60-a872-c0ddf9eb0781' WHERE id = '60681084-fd67-4d40-be40-36d7aa0e4541' AND dedup_key = 'rekey-0025:60681084-fd67-4d40-be40-36d7aa0e4541';
UPDATE internship_postings SET dedup_key = 'ats:workday:copart.wd12:JR101510' WHERE id = 'e6a328d3-7c0e-4af9-8c13-ee7a7a82b988' AND dedup_key = 'rekey-0025:e6a328d3-7c0e-4af9-8c13-ee7a7a82b988';
UPDATE internship_postings SET dedup_key = 'ats:workday:intel.wd1:JR0282641' WHERE id = 'd71cdf89-f425-4ffb-a219-218083431972' AND dedup_key = 'rekey-0025:d71cdf89-f425-4ffb-a219-218083431972';
UPDATE internship_postings SET dedup_key = 'ats:workday:campbellsoup.wd5:Req-65847' WHERE id = '405ade5a-0bcd-47f2-b8d3-ee1e36258650' AND dedup_key = 'rekey-0025:405ade5a-0bcd-47f2-b8d3-ee1e36258650';
UPDATE internship_postings SET dedup_key = 'ats:workday:campbellsoup.wd5:Req-65843' WHERE id = '5d236eab-47c9-4073-829d-43c0b577f61a' AND dedup_key = 'rekey-0025:5d236eab-47c9-4073-829d-43c0b577f61a';
UPDATE internship_postings SET dedup_key = 'ats:workday:synchronyfinancial.wd5:2601751-1' WHERE id = 'be26d2d9-73a7-4318-af81-ad3e867dc5b3' AND dedup_key = 'rekey-0025:be26d2d9-73a7-4318-af81-ad3e867dc5b3';
UPDATE internship_postings SET dedup_key = 'ats:workday:campbellsoup.wd5:Req-65842' WHERE id = '53f08158-0c42-4057-855c-f7a23ba6cb85' AND dedup_key = 'rekey-0025:53f08158-0c42-4057-855c-f7a23ba6cb85';
UPDATE internship_postings SET dedup_key = 'ats:workday:campbellsoup.wd5:Req-66015' WHERE id = '93d015c4-bed6-43dc-809b-f4f107461935' AND dedup_key = 'rekey-0025:93d015c4-bed6-43dc-809b-f4f107461935';
UPDATE internship_postings SET dedup_key = 'ats:workday:cgg.wd103:JR101336-1' WHERE id = 'bd5b0fb9-a560-411e-be46-72354e506727' AND dedup_key = 'rekey-0025:bd5b0fb9-a560-411e-be46-72354e506727';
UPDATE internship_postings SET dedup_key = 'ats:workday:sonyglobal.wd1:JR-119282' WHERE id = 'aa673bf5-b7b0-4b4b-afc3-d4f59da1ab79' AND dedup_key = 'rekey-0025:aa673bf5-b7b0-4b4b-afc3-d4f59da1ab79';
UPDATE internship_postings SET dedup_key = 'ats:workday:nelnet.wd1:R22763' WHERE id = '363c06c9-9298-4a37-9e31-b30b0709fcbe' AND dedup_key = 'rekey-0025:363c06c9-9298-4a37-9e31-b30b0709fcbe';
UPDATE internship_postings SET dedup_key = 'ats:workday:copart.wd12:JR109636' WHERE id = '1e491cd2-24d3-4abf-8c28-1fbf676435fd' AND dedup_key = 'rekey-0025:1e491cd2-24d3-4abf-8c28-1fbf676435fd';
UPDATE internship_postings SET dedup_key = 'ats:workday:paloaltonetworks.wd5:JR-011589' WHERE id = 'f20c8e21-89c9-4d71-8ae6-3263c2b0b5a4' AND dedup_key = 'rekey-0025:f20c8e21-89c9-4d71-8ae6-3263c2b0b5a4';
UPDATE internship_postings SET dedup_key = 'ats:workday:paloaltonetworks.wd5:JR-011605' WHERE id = 'e6fb1aab-3ef8-4006-a7ba-d677280c7aee' AND dedup_key = 'rekey-0025:e6fb1aab-3ef8-4006-a7ba-d677280c7aee';
UPDATE internship_postings SET dedup_key = 'ats:workday:magna.wd3:R00248460' WHERE id = '4d64f71a-68c6-4d0e-a161-7092470ba03b' AND dedup_key = 'rekey-0025:4d64f71a-68c6-4d0e-a161-7092470ba03b';
UPDATE internship_postings SET dedup_key = 'ats:workday:hitachi.wd1:R1013034-1' WHERE id = '56655e51-345b-4f47-9a40-1516ee5af4fc' AND dedup_key = 'rekey-0025:56655e51-345b-4f47-9a40-1516ee5af4fc';
UPDATE internship_postings SET dedup_key = 'ats:workday:arrowstreetcapital.wd5:R1506' WHERE id = '023b5314-fbde-410b-804c-a7842f4e5ae6' AND dedup_key = 'rekey-0025:023b5314-fbde-410b-804c-a7842f4e5ae6';
UPDATE internship_postings SET dedup_key = 'ats:workday:pg.wd5:R000155305' WHERE id = 'fa2dc8ba-fb3f-41bc-8a7a-9531d7965bf0' AND dedup_key = 'rekey-0025:fa2dc8ba-fb3f-41bc-8a7a-9531d7965bf0';
UPDATE internship_postings SET dedup_key = 'ats:workday:revvity.wd103:JR-044468' WHERE id = '87b8c126-e7fe-429f-8bcc-ea60500ea07a' AND dedup_key = 'rekey-0025:87b8c126-e7fe-429f-8bcc-ea60500ea07a';
UPDATE internship_postings SET dedup_key = 'ats:workday:geaerospace.wd5:R5037093-1' WHERE id = '66d9e680-fe24-4be7-be6c-627d6352a9fd' AND dedup_key = 'rekey-0025:66d9e680-fe24-4be7-be6c-627d6352a9fd';
UPDATE internship_postings SET dedup_key = 'ats:workday:geaerospace.wd5:R5037092-1' WHERE id = 'c8c7ca59-1c14-45c0-8de6-a931c46c2e97' AND dedup_key = 'rekey-0025:c8c7ca59-1c14-45c0-8de6-a931c46c2e97';
UPDATE internship_postings SET dedup_key = 'ats:workday:copart.wd12:JR106129' WHERE id = 'c6b1411e-cf87-44dd-852e-200133c110f3' AND dedup_key = 'rekey-0025:c6b1411e-cf87-44dd-852e-200133c110f3';
UPDATE internship_postings SET dedup_key = 'ats:workday:cat.wd5:R0000382293' WHERE id = '2c09ffb6-aed8-4a49-89d6-1bf192bb496f' AND dedup_key = 'rekey-0025:2c09ffb6-aed8-4a49-89d6-1bf192bb496f';
UPDATE internship_postings SET dedup_key = 'ats:workday:chevron.wd5:R000072398-1' WHERE id = '23a77ccd-da2a-4e89-8a1a-f2d03b0544b5' AND dedup_key = 'rekey-0025:23a77ccd-da2a-4e89-8a1a-f2d03b0544b5';
UPDATE internship_postings SET dedup_key = 'ats:workday:ensemblehp.wd5:R048023' WHERE id = 'edc63891-7dd5-4c1b-b943-b1063fc89404' AND dedup_key = 'rekey-0025:edc63891-7dd5-4c1b-b943-b1063fc89404';
UPDATE internship_postings SET dedup_key = 'ats:workday:magna.wd3:R00252238' WHERE id = '176a4755-dac5-4af2-b49f-2213a0aaccb7' AND dedup_key = 'rekey-0025:176a4755-dac5-4af2-b49f-2213a0aaccb7';
UPDATE internship_postings SET dedup_key = 'ats:rippling:denari:8aca4674-f7de-4afa-b031-41c77c533282' WHERE id = '2af51f42-a59d-43db-aac2-eaf939612133' AND dedup_key = 'rekey-0025:2af51f42-a59d-43db-aac2-eaf939612133';
UPDATE internship_postings SET dedup_key = 'ats:rippling:onware:1b9d59b6-1ab0-4c40-8429-39b5b62f019a' WHERE id = '5354c43d-acdf-4353-ac6d-a3a036e5273d' AND dedup_key = 'rekey-0025:5354c43d-acdf-4353-ac6d-a3a036e5273d';
UPDATE internship_postings SET dedup_key = 'ats:rippling:spreeai:aa087086-dd4b-42be-a499-051546655e97' WHERE id = '18f23050-605c-473f-a89e-9c1691f0fd89' AND dedup_key = 'rekey-0025:18f23050-605c-473f-a89e-9c1691f0fd89';
UPDATE internship_postings SET dedup_key = 'ats:rippling:gitar-careers:bfc2d948-40d8-4479-9885-fd1619ec2bda' WHERE id = '2000b0aa-0d1a-4760-85cf-6d7b764f4db9' AND dedup_key = 'rekey-0025:2000b0aa-0d1a-4760-85cf-6d7b764f4db9';
UPDATE internship_postings SET dedup_key = 'ats:workday:flir.wd1:REQ29119' WHERE id = '4a7dfbcd-f80e-447e-b749-005d6cbc977d' AND dedup_key = 'rekey-0025:4a7dfbcd-f80e-447e-b749-005d6cbc977d';
UPDATE internship_postings SET dedup_key = 'ats:workday:ambarella.wd108:JR100361' WHERE id = 'e1fa10dd-e30b-4ecc-92e8-0fef63b76ad8' AND dedup_key = 'rekey-0025:e1fa10dd-e30b-4ecc-92e8-0fef63b76ad8';
UPDATE internship_postings SET dedup_key = 'ats:workday:marvell.wd1:2502424' WHERE id = 'd1e0b402-1734-48a2-b602-92ca589c857c' AND dedup_key = 'rekey-0025:d1e0b402-1734-48a2-b602-92ca589c857c';
UPDATE internship_postings SET dedup_key = 'ats:workday:geaerospace.wd5:R5035583-1' WHERE id = 'a92f9c30-3378-4707-98c1-22ca37af4f78' AND dedup_key = 'rekey-0025:a92f9c30-3378-4707-98c1-22ca37af4f78';
UPDATE internship_postings SET dedup_key = 'ats:workday:boeing.wd1:JR2026516292' WHERE id = 'b8c7c92a-3dd7-4a3a-ad5d-d3f1560036ab' AND dedup_key = 'rekey-0025:b8c7c92a-3dd7-4a3a-ad5d-d3f1560036ab';
UPDATE internship_postings SET dedup_key = 'ats:workday:microchiphr.wd5:R3077-26' WHERE id = '33c98a9b-268c-4e9a-b160-6c041f6a6b94' AND dedup_key = 'rekey-0025:33c98a9b-268c-4e9a-b160-6c041f6a6b94';
UPDATE internship_postings SET dedup_key = 'ats:workday:nidec.wd1:R0016732' WHERE id = '116f29e7-92a8-46aa-850c-afb370b5ed47' AND dedup_key = 'rekey-0025:116f29e7-92a8-46aa-850c-afb370b5ed47';
UPDATE internship_postings SET dedup_key = 'ats:workday:osv-cci.wd1:R1347' WHERE id = 'd20bb200-a808-403a-be7e-024e6297078f' AND dedup_key = 'rekey-0025:d20bb200-a808-403a-be7e-024e6297078f';
UPDATE internship_postings SET dedup_key = 'ats:workday:osv-cci.wd1:R1345' WHERE id = 'd393454d-ddb8-4cb2-8465-312a99131386' AND dedup_key = 'rekey-0025:d393454d-ddb8-4cb2-8465-312a99131386';
UPDATE internship_postings SET dedup_key = 'ats:workday:gresearch.wd103:R3682' WHERE id = '9e76a493-9f87-4754-8cdf-5adef8ee71ea' AND dedup_key = 'rekey-0025:9e76a493-9f87-4754-8cdf-5adef8ee71ea';
UPDATE internship_postings SET dedup_key = 'ats:workday:magna.wd3:R00253444-1' WHERE id = '3a4ccaec-4cdb-4856-9915-a192e1cd01dc' AND dedup_key = 'rekey-0025:3a4ccaec-4cdb-4856-9915-a192e1cd01dc';
UPDATE internship_postings SET dedup_key = 'ats:workday:psu.wd1:REQ_0000080335-1' WHERE id = '7cfd63b6-f6e5-4b8e-a8ff-4c7c447a8f58' AND dedup_key = 'rekey-0025:7cfd63b6-f6e5-4b8e-a8ff-4c7c447a8f58';
UPDATE internship_postings SET dedup_key = 'ats:workday:ambarella.wd108:JR100357' WHERE id = 'e118b30a-682c-419e-b7a3-98e88ea922c2' AND dedup_key = 'rekey-0025:e118b30a-682c-419e-b7a3-98e88ea922c2';
UPDATE internship_postings SET dedup_key = 'ats:workday:ambarella.wd108:JR100364' WHERE id = '3c594b30-149e-43db-b06f-03c5e68cc558' AND dedup_key = 'rekey-0025:3c594b30-149e-43db-b06f-03c5e68cc558';
UPDATE internship_postings SET dedup_key = 'ats:workday:ambarella.wd108:JR100363' WHERE id = 'e95953ea-f630-423f-b196-f52d99ed5fe1' AND dedup_key = 'rekey-0025:e95953ea-f630-423f-b196-f52d99ed5fe1';
UPDATE internship_postings SET dedup_key = 'ats:workday:axiscapital.wd1:REQ06664-1' WHERE id = 'aeada356-563c-4ad3-b7b7-682598c759aa' AND dedup_key = 'rekey-0025:aeada356-563c-4ad3-b7b7-682598c759aa';
UPDATE internship_postings SET dedup_key = 'ats:workday:magna.wd3:R00252235' WHERE id = '52fd037b-5f52-498d-b8df-1a7138a59834' AND dedup_key = 'rekey-0025:52fd037b-5f52-498d-b8df-1a7138a59834';
UPDATE internship_postings SET dedup_key = 'ats:workday:aptiv.wd5:J000700242' WHERE id = 'd8c33668-72ed-456b-9508-5de3fbca4922' AND dedup_key = 'rekey-0025:d8c33668-72ed-456b-9508-5de3fbca4922';
UPDATE internship_postings SET dedup_key = 'ats:rippling:lightguide:37069bc9-a6ba-4841-97fb-b7d743826af0' WHERE id = '392efca9-0296-45cd-b811-45fb02d606b0' AND dedup_key = 'rekey-0025:392efca9-0296-45cd-b811-45fb02d606b0';
UPDATE internship_postings SET dedup_key = 'ats:workday:marmon.wd501:JR0000037451-2' WHERE id = '50a6159d-4dc0-4e45-a7ea-6dee4cd2d830' AND dedup_key = 'rekey-0025:50a6159d-4dc0-4e45-a7ea-6dee4cd2d830';
UPDATE internship_postings SET dedup_key = 'ats:workday:microchiphr.wd5:R2124-25' WHERE id = 'b7822c23-671a-4eec-b132-779630ea44cb' AND dedup_key = 'rekey-0025:b7822c23-671a-4eec-b132-779630ea44cb';
UPDATE internship_postings SET dedup_key = 'ats:workday:nshe.wd1:R0152288' WHERE id = '938d2397-8d81-4f99-abff-f3b25631ad8d' AND dedup_key = 'rekey-0025:938d2397-8d81-4f99-abff-f3b25631ad8d';
UPDATE internship_postings SET dedup_key = 'ats:workday:hendrick.wd5:R-81647' WHERE id = '9e152168-e7cc-4b0b-93c6-1ff97993e1bf' AND dedup_key = 'rekey-0025:9e152168-e7cc-4b0b-93c6-1ff97993e1bf';
UPDATE internship_postings SET dedup_key = 'ats:workday:medtronic.wd1:R73630' WHERE id = '2aa8a486-104e-4dc0-9368-9b1813a1bc9c' AND dedup_key = 'rekey-0025:2aa8a486-104e-4dc0-9368-9b1813a1bc9c';
UPDATE internship_postings SET dedup_key = 'ats:workday:ciena.wd5:R031443' WHERE id = '52661293-1fe9-48e3-bb00-2a4236e8412e' AND dedup_key = 'rekey-0025:52661293-1fe9-48e3-bb00-2a4236e8412e';
UPDATE internship_postings SET dedup_key = 'ats:workday:novanta.wd5:R009487' WHERE id = '972fc179-8a8a-4d89-8566-0d40938ba308' AND dedup_key = 'rekey-0025:972fc179-8a8a-4d89-8566-0d40938ba308';
UPDATE internship_postings SET dedup_key = 'ats:workday:novanta.wd5:R009484' WHERE id = '7c2bbdda-7f8c-4ff7-8905-7c5f56d41e90' AND dedup_key = 'rekey-0025:7c2bbdda-7f8c-4ff7-8905-7c5f56d41e90';
UPDATE internship_postings SET dedup_key = 'ats:workday:uline.wd1:R265685' WHERE id = 'bb8fd547-4994-422a-9600-cd6a12d50c24' AND dedup_key = 'rekey-0025:bb8fd547-4994-422a-9600-cd6a12d50c24';
UPDATE internship_postings SET dedup_key = 'ats:workday:uline.wd1:R265684' WHERE id = 'e16606e2-c732-400a-ab56-a58598208013' AND dedup_key = 'rekey-0025:e16606e2-c732-400a-ab56-a58598208013';
UPDATE internship_postings SET dedup_key = 'ats:workday:microchiphr.wd5:R3371-26' WHERE id = 'aa807a96-76ed-4ea7-8f79-f9fc6c1de6d9' AND dedup_key = 'rekey-0025:aa807a96-76ed-4ea7-8f79-f9fc6c1de6d9';
UPDATE internship_postings SET dedup_key = 'ats:workable:ecareers:C82B9AD635' WHERE id = '93f71ba0-ee1e-4a31-b2cb-1eff60040968' AND dedup_key = 'rekey-0025:93f71ba0-ee1e-4a31-b2cb-1eff60040968';
UPDATE internship_postings SET dedup_key = 'ats:workday:nidec.wd1:R0017023' WHERE id = '8786006c-cf5b-496b-ae78-a82ee6a33db2' AND dedup_key = 'rekey-0025:8786006c-cf5b-496b-ae78-a82ee6a33db2';
UPDATE internship_postings SET dedup_key = 'ats:workday:arlo.wd12:JR100404' WHERE id = '65001e0b-97b4-423d-8c46-59adca38173a' AND dedup_key = 'rekey-0025:65001e0b-97b4-423d-8c46-59adca38173a';
UPDATE internship_postings SET dedup_key = 'ats:workday:haier.wd3:REQ-26431' WHERE id = '612b1a18-9949-44ba-a248-162e7f7760fe' AND dedup_key = 'rekey-0025:612b1a18-9949-44ba-a248-162e7f7760fe';
UPDATE internship_postings SET dedup_key = 'ats:workday:haier.wd3:REQ-26427' WHERE id = '7e9f8453-26a9-4db7-9ffd-f4762800543e' AND dedup_key = 'rekey-0025:7e9f8453-26a9-4db7-9ffd-f4762800543e';
UPDATE internship_postings SET dedup_key = 'ats:workday:ngc.wd1:R10242395' WHERE id = '8b2484bd-b047-44ba-b28e-bef9d53e0554' AND dedup_key = 'rekey-0025:8b2484bd-b047-44ba-b28e-bef9d53e0554';
UPDATE internship_postings SET dedup_key = 'ats:workday:selinc.wd1:2026-22385' WHERE id = 'c05f57a4-8119-4dd9-9ccc-0bfa6139c0ff' AND dedup_key = 'rekey-0025:c05f57a4-8119-4dd9-9ccc-0bfa6139c0ff';
UPDATE internship_postings SET dedup_key = 'ats:workable:luminance-1:E045EF5A7A' WHERE id = '9ec2f6cd-3995-4ee3-b44b-ee7ad53b86a2' AND dedup_key = 'rekey-0025:9ec2f6cd-3995-4ee3-b44b-ee7ad53b86a2';
UPDATE internship_postings SET dedup_key = 'ats:workday:ciena.wd5:R031492' WHERE id = 'bb097eea-c221-4421-b843-850af0c02d12' AND dedup_key = 'rekey-0025:bb097eea-c221-4421-b843-850af0c02d12';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01863596' WHERE id = 'd1dd7318-920b-4a2a-b059-153d3ff44473' AND dedup_key = 'rekey-0025:d1dd7318-920b-4a2a-b059-153d3ff44473';
UPDATE internship_postings SET dedup_key = 'ats:workday:amcor.wd5:REQ_94654' WHERE id = '036f438d-a732-4e74-bea2-fdb40e95ebf2' AND dedup_key = 'rekey-0025:036f438d-a732-4e74-bea2-fdb40e95ebf2';
UPDATE internship_postings SET dedup_key = 'ats:greenhouse:gh_jid:5207089007' WHERE id = '8589d864-545a-4875-9951-68561a6d71aa' AND dedup_key = 'rekey-0025:8589d864-545a-4875-9951-68561a6d71aa';
UPDATE internship_postings SET dedup_key = 'ats:workday:autodesk.wd1:26WD100400' WHERE id = 'f8657107-e829-40e2-828b-a0607ac89aba' AND dedup_key = 'rekey-0025:f8657107-e829-40e2-828b-a0607ac89aba';
UPDATE internship_postings SET dedup_key = 'ats:workday:autodesk.wd1:26WD100406-2' WHERE id = '6cc3cd64-826f-4480-b6ba-48a67ba88a1f' AND dedup_key = 'rekey-0025:6cc3cd64-826f-4480-b6ba-48a67ba88a1f';
UPDATE internship_postings SET dedup_key = 'ats:workable:pronexus-1:AF8C34AC6D' WHERE id = 'd1820183-6213-48bc-b879-5999c444110e' AND dedup_key = 'rekey-0025:d1820183-6213-48bc-b879-5999c444110e';
UPDATE internship_postings SET dedup_key = 'ats:workday:bah.wd1:R0246415' WHERE id = 'e63afb76-2f37-4a79-a54c-f386ce3888f0' AND dedup_key = 'rekey-0025:e63afb76-2f37-4a79-a54c-f386ce3888f0';
UPDATE internship_postings SET dedup_key = 'ats:workday:lplfinancial.wd1:R-052921' WHERE id = '0d43f402-f060-46b2-9f16-f46d444461b0' AND dedup_key = 'rekey-0025:0d43f402-f060-46b2-9f16-f46d444461b0';
UPDATE internship_postings SET dedup_key = 'ats:workday:lplfinancial.wd1:R-052914' WHERE id = '033a421e-ed3d-4d23-ba0f-0704938c9190' AND dedup_key = 'rekey-0025:033a421e-ed3d-4d23-ba0f-0704938c9190';
UPDATE internship_postings SET dedup_key = 'ats:workable:trycaddi:9D1291C697' WHERE id = '3ebfc8b8-393a-4284-a774-ee52bd843243' AND dedup_key = 'rekey-0025:3ebfc8b8-393a-4284-a774-ee52bd843243';
UPDATE internship_postings SET dedup_key = 'ats:workable:responsiveads-inc:493EAC12D6' WHERE id = 'b63134ea-00a7-4c0b-84af-c2789c8b0f12' AND dedup_key = 'rekey-0025:b63134ea-00a7-4c0b-84af-c2789c8b0f12';
UPDATE internship_postings SET dedup_key = 'ats:workday:micron.wd1:JR108471' WHERE id = '94e430d1-2eaa-4bd0-b125-f0ccd67af3a6' AND dedup_key = 'rekey-0025:94e430d1-2eaa-4bd0-b125-f0ccd67af3a6';
UPDATE internship_postings SET dedup_key = 'ats:workday:micron.wd1:JR108458' WHERE id = '3b22dd22-db7b-4ded-8732-2934ca26e2b6' AND dedup_key = 'rekey-0025:3b22dd22-db7b-4ded-8732-2934ca26e2b6';
UPDATE internship_postings SET dedup_key = 'ats:workday:autodesk.wd1:26WD100398-2' WHERE id = '9b96bd34-9652-45e3-a7f4-355e3e07541d' AND dedup_key = 'rekey-0025:9b96bd34-9652-45e3-a7f4-355e3e07541d';
UPDATE internship_postings SET dedup_key = 'ats:workday:autodesk.wd1:26WD100398-1' WHERE id = 'a5ef2631-5ee3-4d43-ae47-a26ed56a1496' AND dedup_key = 'rekey-0025:a5ef2631-5ee3-4d43-ae47-a26ed56a1496';
UPDATE internship_postings SET dedup_key = 'ats:greenhouse:gh_jid:4341038009' WHERE id = 'fb5b75a2-094d-4156-9c83-52a8da162b30' AND dedup_key = 'rekey-0025:fb5b75a2-094d-4156-9c83-52a8da162b30';
UPDATE internship_postings SET dedup_key = 'ats:greenhouse:gh_jid:4340833009' WHERE id = '00f5a0eb-c6b7-4947-b032-4e61d1eab869' AND dedup_key = 'rekey-0025:00f5a0eb-c6b7-4947-b032-4e61d1eab869';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01867943' WHERE id = '323eba0f-e8f2-4334-980f-557aa710ffd5' AND dedup_key = 'rekey-0025:323eba0f-e8f2-4334-980f-557aa710ffd5';
UPDATE internship_postings SET dedup_key = 'ats:workday:blueorigin.wd5:R70275' WHERE id = 'd6047404-5634-407b-a133-09f151397e55' AND dedup_key = 'rekey-0025:d6047404-5634-407b-a133-09f151397e55';
UPDATE internship_postings SET dedup_key = 'ats:workable:tmeic-corporation-americas:68E556E5CA' WHERE id = 'e810753f-fd0b-415c-ad19-6630e340171b' AND dedup_key = 'rekey-0025:e810753f-fd0b-415c-ad19-6630e340171b';
UPDATE internship_postings SET dedup_key = 'ats:workable:tmeic-corporation-americas:6FDBF2FD32' WHERE id = 'a8aad92d-a430-471c-beaa-f1aa87bf0be4' AND dedup_key = 'rekey-0025:a8aad92d-a430-471c-beaa-f1aa87bf0be4';
UPDATE internship_postings SET dedup_key = 'ats:workday:americanfidelity.wd5:JR1005' WHERE id = '10be6784-c1d1-4671-a2c2-b98652c4e36a' AND dedup_key = 'rekey-0025:10be6784-c1d1-4671-a2c2-b98652c4e36a';
UPDATE internship_postings SET dedup_key = 'ats:workday:selinc.wd1:2026-22601' WHERE id = '0d3549ae-b826-469f-8a99-a59ccaaa1524' AND dedup_key = 'rekey-0025:0d3549ae-b826-469f-8a99-a59ccaaa1524';
UPDATE internship_postings SET dedup_key = 'ats:workday:selinc.wd1:2026-22361' WHERE id = '051431cd-7a40-4b3b-8bbd-3b86f64e6117' AND dedup_key = 'rekey-0025:051431cd-7a40-4b3b-8bbd-3b86f64e6117';
UPDATE internship_postings SET dedup_key = 'ats:workday:micron.wd1:JR108533' WHERE id = '71572d77-c807-45cf-9dc9-43bcc921590a' AND dedup_key = 'rekey-0025:71572d77-c807-45cf-9dc9-43bcc921590a';
UPDATE internship_postings SET dedup_key = 'ats:workday:crowe.wd12:R-51782' WHERE id = 'fc1148ef-ad2f-4e02-a224-76de2a6d45fa' AND dedup_key = 'rekey-0025:fc1148ef-ad2f-4e02-a224-76de2a6d45fa';
UPDATE internship_postings SET dedup_key = 'ats:workday:flir.wd1:REQ36194-2' WHERE id = 'd0d8967b-2abd-4c4c-9761-9847d6c5879c' AND dedup_key = 'rekey-0025:d0d8967b-2abd-4c4c-9761-9847d6c5879c';
UPDATE internship_postings SET dedup_key = 'ats:workday:bakerhughes.wd5:R168066' WHERE id = '7d76d7fb-2d40-4444-acf5-572eb8924736' AND dedup_key = 'rekey-0025:7d76d7fb-2d40-4444-acf5-572eb8924736';
UPDATE internship_postings SET dedup_key = 'ats:workday:generac.wd5:JR16149' WHERE id = 'abd2a4b0-ef92-45b2-8b04-d8fd73f5ee6a' AND dedup_key = 'rekey-0025:abd2a4b0-ef92-45b2-8b04-d8fd73f5ee6a';
UPDATE internship_postings SET dedup_key = 'ats:workday:motorolasolutions.wd5:R67362-1' WHERE id = '36b5c5c0-1e50-4be6-aefe-919447275458' AND dedup_key = 'rekey-0025:36b5c5c0-1e50-4be6-aefe-919447275458';
UPDATE internship_postings SET dedup_key = 'ats:workday:intel.wd1:JR0283509' WHERE id = '5e5cc234-2f69-4bf9-959f-ac7ef7348c15' AND dedup_key = 'rekey-0025:5e5cc234-2f69-4bf9-959f-ac7ef7348c15';
UPDATE internship_postings SET dedup_key = 'ats:workday:interdigital.wd5:REQ26-1135' WHERE id = '8a3b10ee-aaf4-4690-8c86-932e71d46bba' AND dedup_key = 'rekey-0025:8a3b10ee-aaf4-4690-8c86-932e71d46bba';
UPDATE internship_postings SET dedup_key = 'ats:workday:oneok.wd1:R8155' WHERE id = '90235642-e064-4846-bbc1-34f8f4f46fbd' AND dedup_key = 'rekey-0025:90235642-e064-4846-bbc1-34f8f4f46fbd';
UPDATE internship_postings SET dedup_key = 'ats:workday:crowe.wd12:R-71005' WHERE id = '84c8bf16-7159-4b62-a1e5-c016286f4e53' AND dedup_key = 'rekey-0025:84c8bf16-7159-4b62-a1e5-c016286f4e53';
UPDATE internship_postings SET dedup_key = 'ats:rippling:boom-supersonic:4053f749-3d30-4d71-a64c-5fa79e162bbf' WHERE id = '5b56a4b6-a845-4e12-8971-0350a548d4d2' AND dedup_key = 'rekey-0025:5b56a4b6-a845-4e12-8971-0350a548d4d2';
UPDATE internship_postings SET dedup_key = 'ats:workday:nxp.wd3:R-10065882-1' WHERE id = '79e60b2a-762b-4ddc-aefa-9fd4591031a1' AND dedup_key = 'rekey-0025:79e60b2a-762b-4ddc-aefa-9fd4591031a1';
UPDATE internship_postings SET dedup_key = 'ats:workday:geaerospace.wd5:R5029637-1' WHERE id = '03dd05c3-751a-42b5-84bb-3ec2dcec6d3d' AND dedup_key = 'rekey-0025:03dd05c3-751a-42b5-84bb-3ec2dcec6d3d';
UPDATE internship_postings SET dedup_key = 'ats:workday:suncor.wd1:R0017525' WHERE id = '2c3dee96-ac3d-449e-b012-b573959e1772' AND dedup_key = 'rekey-0025:2c3dee96-ac3d-449e-b012-b573959e1772';
UPDATE internship_postings SET dedup_key = 'ats:workday:capitalone.wd12:R249010' WHERE id = '0e26bf56-32c8-43e2-bad4-d09add5608b7' AND dedup_key = 'rekey-0025:0e26bf56-32c8-43e2-bad4-d09add5608b7';
UPDATE internship_postings SET dedup_key = 'ats:workday:deezee.wd108:REQ00368' WHERE id = '61225403-6321-4ccb-a6c3-8acf224da672' AND dedup_key = 'rekey-0025:61225403-6321-4ccb-a6c3-8acf224da672';
UPDATE internship_postings SET dedup_key = 'ats:workday:americanfidelity.wd5:JR1021' WHERE id = 'e900c485-7392-4f64-a4d5-185c8382c782' AND dedup_key = 'rekey-0025:e900c485-7392-4f64-a4d5-185c8382c782';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184501-1' WHERE id = '07822e9c-aa5a-4684-9cc2-7e375562a802' AND dedup_key = 'rekey-0025:07822e9c-aa5a-4684-9cc2-7e375562a802';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184599-1' WHERE id = '697b1cd2-6776-48c5-a7db-d95d2f6bfa08' AND dedup_key = 'rekey-0025:697b1cd2-6776-48c5-a7db-d95d2f6bfa08';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184676-1' WHERE id = 'fabdba89-b6db-474e-9506-89eff3e6460d' AND dedup_key = 'rekey-0025:fabdba89-b6db-474e-9506-89eff3e6460d';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184491-1' WHERE id = '5575d8d2-4bc9-4f53-8e40-91f9dc6dbc50' AND dedup_key = 'rekey-0025:5575d8d2-4bc9-4f53-8e40-91f9dc6dbc50';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184603-2' WHERE id = 'bc52bac6-2467-475e-8a16-0b3989d5bd56' AND dedup_key = 'rekey-0025:bc52bac6-2467-475e-8a16-0b3989d5bd56';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184599' WHERE id = 'e0a82643-7d09-47bb-a347-33c56f4ab6bd' AND dedup_key = 'rekey-0025:e0a82643-7d09-47bb-a347-33c56f4ab6bd';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184555' WHERE id = '8286b617-5020-455c-bd64-13a3feb6e561' AND dedup_key = 'rekey-0025:8286b617-5020-455c-bd64-13a3feb6e561';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184555-1' WHERE id = '418834cf-1a1e-4855-819b-87078c72192f' AND dedup_key = 'rekey-0025:418834cf-1a1e-4855-819b-87078c72192f';
UPDATE internship_postings SET dedup_key = 'ats:workday:intel.wd1:JR0285451-1' WHERE id = '021075d5-0792-48de-a18b-338fdf2bdc62' AND dedup_key = 'rekey-0025:021075d5-0792-48de-a18b-338fdf2bdc62';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184676' WHERE id = 'a369e9ac-8462-4378-aca3-1ccb4270af43' AND dedup_key = 'rekey-0025:a369e9ac-8462-4378-aca3-1ccb4270af43';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184499' WHERE id = '6fc84751-2310-4f42-8a4f-9796d1910b91' AND dedup_key = 'rekey-0025:6fc84751-2310-4f42-8a4f-9796d1910b91';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184491' WHERE id = '3cdf5694-2ddd-4d62-85a1-c0da0046476a' AND dedup_key = 'rekey-0025:3cdf5694-2ddd-4d62-85a1-c0da0046476a';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184830' WHERE id = 'b389bb41-7ce1-4af3-bb25-d379421eb44f' AND dedup_key = 'rekey-0025:b389bb41-7ce1-4af3-bb25-d379421eb44f';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000184501' WHERE id = 'c161a7b6-c1f4-450e-8c36-8e4d7c16bd62' AND dedup_key = 'rekey-0025:c161a7b6-c1f4-450e-8c36-8e4d7c16bd62';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01866850' WHERE id = 'd1ad8c66-5fc9-410b-9704-5b7e0e2eaa54' AND dedup_key = 'rekey-0025:d1ad8c66-5fc9-410b-9704-5b7e0e2eaa54';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01871423' WHERE id = 'c973db42-1962-408a-a53b-2e38d1633413' AND dedup_key = 'rekey-0025:c973db42-1962-408a-a53b-2e38d1633413';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01866914' WHERE id = '35041534-47c7-4a88-90bb-505503e80efa' AND dedup_key = 'rekey-0025:35041534-47c7-4a88-90bb-505503e80efa';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01864698' WHERE id = '0a3a2701-2766-48d8-8f5b-f7b820fbb651' AND dedup_key = 'rekey-0025:0a3a2701-2766-48d8-8f5b-f7b820fbb651';
UPDATE internship_postings SET dedup_key = 'ats:workday:draper.wd5:JR002832-1' WHERE id = '02784837-f744-4ff0-ba35-4ae38c0ba9ba' AND dedup_key = 'rekey-0025:02784837-f744-4ff0-ba35-4ae38c0ba9ba';
UPDATE internship_postings SET dedup_key = 'ats:workday:foresters.wd3:R-2305' WHERE id = '05bca639-db14-4fe1-ad3e-cc7ac2d6c70e' AND dedup_key = 'rekey-0025:05bca639-db14-4fe1-ad3e-cc7ac2d6c70e';
UPDATE internship_postings SET dedup_key = 'ats:workday:amcor.wd5:REQ_93190' WHERE id = '9cc995a4-abd2-41cf-8b26-14851ed7284b' AND dedup_key = 'rekey-0025:9cc995a4-abd2-41cf-8b26-14851ed7284b';
UPDATE internship_postings SET dedup_key = 'ats:workday:pimco.wd1:R106800' WHERE id = '9a5ba322-f3fb-4055-9527-b8614c330dbf' AND dedup_key = 'rekey-0025:9a5ba322-f3fb-4055-9527-b8614c330dbf';
UPDATE internship_postings SET dedup_key = 'ats:workday:nvidia.wd5:JR2023492' WHERE id = 'b1dad54a-12b6-4da6-8865-bfb183e0fbf5' AND dedup_key = 'rekey-0025:b1dad54a-12b6-4da6-8865-bfb183e0fbf5';
UPDATE internship_postings SET dedup_key = 'ats:workday:nvidia.wd5:JR2023499' WHERE id = 'f9eb9f37-49a3-4995-a7dc-235f56e019e1' AND dedup_key = 'rekey-0025:f9eb9f37-49a3-4995-a7dc-235f56e019e1';
UPDATE internship_postings SET dedup_key = 'ats:workday:nvidia.wd5:JR2023495' WHERE id = '24ac88e4-c7fc-45fe-911e-6a21c530a2c2' AND dedup_key = 'rekey-0025:24ac88e4-c7fc-45fe-911e-6a21c530a2c2';
UPDATE internship_postings SET dedup_key = 'ats:workday:nvidia.wd5:JR2023856' WHERE id = 'f644d175-0027-4e71-9fe2-9d54eb2052d1' AND dedup_key = 'rekey-0025:f644d175-0027-4e71-9fe2-9d54eb2052d1';
UPDATE internship_postings SET dedup_key = 'ats:workday:copart.wd12:JR110617' WHERE id = '2b70a559-694b-423d-b29c-d95869ca38e1' AND dedup_key = 'rekey-0025:2b70a559-694b-423d-b29c-d95869ca38e1';
UPDATE internship_postings SET dedup_key = 'ats:workday:gevernova.wd5:R5050417-1' WHERE id = '3e973e33-55d6-43ef-a8dd-841744be2c1c' AND dedup_key = 'rekey-0025:3e973e33-55d6-43ef-a8dd-841744be2c1c';
UPDATE internship_postings SET dedup_key = 'ats:workday:moog.wd5:R-26-19378' WHERE id = 'b1915506-fb01-4c6a-8c94-0b59cf847e5f' AND dedup_key = 'rekey-0025:b1915506-fb01-4c6a-8c94-0b59cf847e5f';
UPDATE internship_postings SET dedup_key = 'ats:workday:autodesk.wd1:26WD100523-1' WHERE id = 'd9b21fc8-10e9-46c4-b3d5-fa0a9a18dd9a' AND dedup_key = 'rekey-0025:d9b21fc8-10e9-46c4-b3d5-fa0a9a18dd9a';
UPDATE internship_postings SET dedup_key = 'ats:workday:autodesk.wd1:26WD100523-2' WHERE id = '0ea56dcd-72f2-4a5c-b18b-ae2f896d79fb' AND dedup_key = 'rekey-0025:0ea56dcd-72f2-4a5c-b18b-ae2f896d79fb';
UPDATE internship_postings SET dedup_key = 'ats:workable:tmeic-corporation-americas:532EE44DFB' WHERE id = '338325a9-ef91-4803-b67e-f65a165f66cb' AND dedup_key = 'rekey-0025:338325a9-ef91-4803-b67e-f65a165f66cb';
UPDATE internship_postings SET dedup_key = 'ats:workable:western-magnetics:E366930F3F' WHERE id = '17cbd4f4-7426-44f5-a4c4-f867000fb8b9' AND dedup_key = 'rekey-0025:17cbd4f4-7426-44f5-a4c4-f867000fb8b9';
UPDATE internship_postings SET dedup_key = 'ats:rippling:ampersand-biomedicines:be3f5479-379e-4d48-9bd1-82a69b2dcfd3' WHERE id = 'c80e9d8d-0b0b-4ad5-a5c0-138cfde724d3' AND dedup_key = 'rekey-0025:c80e9d8d-0b0b-4ad5-a5c0-138cfde724d3';
UPDATE internship_postings SET dedup_key = 'ats:workday:sysco.wd5:R263660' WHERE id = '23b73715-0fde-407b-89eb-416252685cb0' AND dedup_key = 'rekey-0025:23b73715-0fde-407b-89eb-416252685cb0';
UPDATE internship_postings SET dedup_key = 'ats:workday:fifththird.wd5:R71588' WHERE id = 'e99496e1-6680-4830-a9e6-8e85c1239353' AND dedup_key = 'rekey-0025:e99496e1-6680-4830-a9e6-8e85c1239353';
UPDATE internship_postings SET dedup_key = 'ats:workday:geico.wd1:R0065372' WHERE id = '0dc5d729-c295-4507-8d4d-0ead2b538939' AND dedup_key = 'rekey-0025:0dc5d729-c295-4507-8d4d-0ead2b538939';
UPDATE internship_postings SET dedup_key = 'ats:workday:geico.wd1:R0065373' WHERE id = '2563d96d-1f63-4deb-8bfe-6bf86750744f' AND dedup_key = 'rekey-0025:2563d96d-1f63-4deb-8bfe-6bf86750744f';
UPDATE internship_postings SET dedup_key = 'ats:workday:sysco.wd5:R263666' WHERE id = 'a7487f11-2dcc-44c2-892a-33c240adab21' AND dedup_key = 'rekey-0025:a7487f11-2dcc-44c2-892a-33c240adab21';
UPDATE internship_postings SET dedup_key = 'ats:workday:devonenergy.wd5:R26264-1' WHERE id = '7fd687d8-5ec3-4529-aeeb-4f11f33531de' AND dedup_key = 'rekey-0025:7fd687d8-5ec3-4529-aeeb-4f11f33531de';
UPDATE internship_postings SET dedup_key = 'ats:workday:lthc.wd1:JR103879-2' WHERE id = '6feef66b-4331-46f1-abdc-bcb0ec4c4a97' AND dedup_key = 'rekey-0025:6feef66b-4331-46f1-abdc-bcb0ec4c4a97';
UPDATE internship_postings SET dedup_key = 'ats:workday:lthc.wd1:JR103878-2' WHERE id = 'a55eb371-1514-4c6b-8ab8-433239b6688f' AND dedup_key = 'rekey-0025:a55eb371-1514-4c6b-8ab8-433239b6688f';
UPDATE internship_postings SET dedup_key = 'ats:workday:eversource.wd1:R-031600' WHERE id = 'e6a732d2-1532-4080-8ebc-c1d3a5c2c841' AND dedup_key = 'rekey-0025:e6a732d2-1532-4080-8ebc-c1d3a5c2c841';
UPDATE internship_postings SET dedup_key = 'ats:workday:micron.wd1:JR109290' WHERE id = 'acedd991-1657-4883-ada0-d60834d9668c' AND dedup_key = 'rekey-0025:acedd991-1657-4883-ada0-d60834d9668c';
UPDATE internship_postings SET dedup_key = 'ats:workday:micron.wd1:JR109154' WHERE id = '0066a591-40c8-45b0-8527-91d29dbeda0d' AND dedup_key = 'rekey-0025:0066a591-40c8-45b0-8527-91d29dbeda0d';
UPDATE internship_postings SET dedup_key = 'ats:workday:analogdevices.wd1:R265299' WHERE id = 'b009f355-954f-472a-9031-aabc41bfafec' AND dedup_key = 'rekey-0025:b009f355-954f-472a-9031-aabc41bfafec';
UPDATE internship_postings SET dedup_key = 'ats:workday:otppb.wd3:7193' WHERE id = 'ea1a034c-79c0-4532-b344-8475294482e2' AND dedup_key = 'rekey-0025:ea1a034c-79c0-4532-b344-8475294482e2';
UPDATE internship_postings SET dedup_key = 'ats:rippling:rippling:3fd9615a-d0c7-458c-a0fc-5d9d7f0ce77c' WHERE id = 'a602301e-83d6-406d-b4ca-972d199a71c2' AND dedup_key = 'rekey-0025:a602301e-83d6-406d-b4ca-972d199a71c2';
UPDATE internship_postings SET dedup_key = 'ats:rippling:rippling:ee1ec0b1-9a55-408d-979d-9c74f257e9ea' WHERE id = 'd4c25f40-dd0a-40ba-9c7c-2352aca539d7' AND dedup_key = 'rekey-0025:d4c25f40-dd0a-40ba-9c7c-2352aca539d7';
UPDATE internship_postings SET dedup_key = 'ats:rippling:rippling:203e0cac-0e30-4603-8087-f764e8c3f85c' WHERE id = '6578318f-23d7-48ac-a4ba-aa88ec455aa6' AND dedup_key = 'rekey-0025:6578318f-23d7-48ac-a4ba-aa88ec455aa6';
UPDATE internship_postings SET dedup_key = 'ats:workable:al-warren-oil-company-inc:A4487B349D' WHERE id = '50be4f72-f56b-46c9-bc13-e3f5091b9a7b' AND dedup_key = 'rekey-0025:50be4f72-f56b-46c9-bc13-e3f5091b9a7b';
UPDATE internship_postings SET dedup_key = 'ats:workday:abb.wd3:JR00038999' WHERE id = '39d12781-895a-48b3-92ad-3ff0593dfe58' AND dedup_key = 'rekey-0025:39d12781-895a-48b3-92ad-3ff0593dfe58';
UPDATE internship_postings SET dedup_key = 'ats:rippling:moon:8b81bca7-1a64-4377-8ea8-869aac03080b' WHERE id = '3fc863a0-b4fd-4e24-9597-af00347400fc' AND dedup_key = 'rekey-0025:3fc863a0-b4fd-4e24-9597-af00347400fc';
UPDATE internship_postings SET dedup_key = 'ats:rippling:greengas:b2938290-cc66-4f54-9888-bbe286c1d9b6' WHERE id = '6d0f3b3a-3dd0-443d-a4f6-24dd612f8a0a' AND dedup_key = 'rekey-0025:6d0f3b3a-3dd0-443d-a4f6-24dd612f8a0a';
UPDATE internship_postings SET dedup_key = 'ats:workable:hyperlight:5581EA0668' WHERE id = 'f0b73ea4-dda6-4f4c-8263-e16fb96619b6' AND dedup_key = 'rekey-0025:f0b73ea4-dda6-4f4c-8263-e16fb96619b6';
UPDATE internship_postings SET dedup_key = 'ats:workday:osv-cci.wd1:R1344' WHERE id = '275b145e-e12e-4b33-83a9-2f16547c8eb8' AND dedup_key = 'rekey-0025:275b145e-e12e-4b33-83a9-2f16547c8eb8';
UPDATE internship_postings SET dedup_key = 'ats:workday:medtronic.wd1:R73630-1' WHERE id = '454f395b-deba-43a5-9d54-739bd0abf4eb' AND dedup_key = 'rekey-0025:454f395b-deba-43a5-9d54-739bd0abf4eb';
UPDATE internship_postings SET dedup_key = 'ats:workday:capitalone.wd12:R244387-1' WHERE id = 'd67999ed-0b2f-4df2-bf0c-7722935a4983' AND dedup_key = 'rekey-0025:d67999ed-0b2f-4df2-bf0c-7722935a4983';
UPDATE internship_postings SET dedup_key = 'ats:workday:ntst.wd1:R015667' WHERE id = 'd8bab602-37da-4a9f-af07-216fad703ece' AND dedup_key = 'rekey-0025:d8bab602-37da-4a9f-af07-216fad703ece';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01863072' WHERE id = '693e1bc8-8b39-4845-9a12-fd80839b3dcd' AND dedup_key = 'rekey-0025:693e1bc8-8b39-4845-9a12-fd80839b3dcd';
UPDATE internship_postings SET dedup_key = 'ats:workday:nvidia.wd5:JR2022295' WHERE id = '6eca1169-369b-4682-9bdb-5179b1c41e16' AND dedup_key = 'rekey-0025:6eca1169-369b-4682-9bdb-5179b1c41e16';
UPDATE internship_postings SET dedup_key = 'ats:workday:gevernova.wd5:R5050022-2' WHERE id = 'de0502bc-ed79-4938-a1b6-31814f85dea1' AND dedup_key = 'rekey-0025:de0502bc-ed79-4938-a1b6-31814f85dea1';
UPDATE internship_postings SET dedup_key = 'ats:workday:gevernova.wd5:R5050023-2' WHERE id = '3860de9a-f93f-4437-8011-cc2deff05397' AND dedup_key = 'rekey-0025:3860de9a-f93f-4437-8011-cc2deff05397';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01866869' WHERE id = 'c5be20d3-2ca8-4c2c-87a3-95807d7875f2' AND dedup_key = 'rekey-0025:c5be20d3-2ca8-4c2c-87a3-95807d7875f2';
UPDATE internship_postings SET dedup_key = 'ats:workday:frostbank.wd5:R261550' WHERE id = 'a751a17b-164f-46ca-b766-b877f51326a0' AND dedup_key = 'rekey-0025:a751a17b-164f-46ca-b766-b877f51326a0';
UPDATE internship_postings SET dedup_key = 'ats:workday:generalmotors.wd5:JR-202618179' WHERE id = '6bcd985c-83dd-463e-af07-1335190f5a6b' AND dedup_key = 'rekey-0025:6bcd985c-83dd-463e-af07-1335190f5a6b';
UPDATE internship_postings SET dedup_key = 'ats:workday:brunswick.wd1:JR-051321' WHERE id = '8cdd0f44-0699-45ce-b250-d0b4b335b674' AND dedup_key = 'rekey-0025:8cdd0f44-0699-45ce-b250-d0b4b335b674';
UPDATE internship_postings SET dedup_key = 'ats:workday:availity.wd1:R0008436' WHERE id = '4c6b86af-d300-44dc-9520-ff588cf77cb6' AND dedup_key = 'rekey-0025:4c6b86af-d300-44dc-9520-ff588cf77cb6';
UPDATE internship_postings SET dedup_key = 'ats:workable:elevate-semiconductor:F234DECA3C' WHERE id = 'ccc3d602-edbb-471f-a5a7-31c86b8e1be1' AND dedup_key = 'rekey-0025:ccc3d602-edbb-471f-a5a7-31c86b8e1be1';
UPDATE internship_postings SET dedup_key = 'ats:workday:flir.wd1:REQ36193' WHERE id = '44c6d0b5-3e6b-4934-a565-9263aa68ccd0' AND dedup_key = 'rekey-0025:44c6d0b5-3e6b-4934-a565-9263aa68ccd0';
UPDATE internship_postings SET dedup_key = 'ats:workday:copart.wd12:JR109490' WHERE id = 'da7a8022-662b-45de-ad38-ca9cf9c8be6c' AND dedup_key = 'rekey-0025:da7a8022-662b-45de-ad38-ca9cf9c8be6c';
UPDATE internship_postings SET dedup_key = 'ats:workday:copart.wd12:JR109671' WHERE id = '980e64a8-3d2c-40c8-934b-9f62ba90e7a3' AND dedup_key = 'rekey-0025:980e64a8-3d2c-40c8-934b-9f62ba90e7a3';
UPDATE internship_postings SET dedup_key = 'ats:workday:humana.wd5:R-424692-1' WHERE id = '460558ae-12f2-4ce7-a60c-bd8eb10879ee' AND dedup_key = 'rekey-0025:460558ae-12f2-4ce7-a60c-bd8eb10879ee';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01865635' WHERE id = '9001e0a5-7256-4b71-8187-e4d435998fa5' AND dedup_key = 'rekey-0025:9001e0a5-7256-4b71-8187-e4d435998fa5';
UPDATE internship_postings SET dedup_key = 'ats:workday:campbellsoup.wd5:Req-66014' WHERE id = '939e2c58-ac83-4853-b09b-f31a420a7ce9' AND dedup_key = 'rekey-0025:939e2c58-ac83-4853-b09b-f31a420a7ce9';
UPDATE internship_postings SET dedup_key = 'ats:workday:campbellsoup.wd5:Req-65838' WHERE id = '8f2d3b53-ea18-4658-acd9-5516294b20c6' AND dedup_key = 'rekey-0025:8f2d3b53-ea18-4658-acd9-5516294b20c6';
UPDATE internship_postings SET dedup_key = 'ats:workday:analogdevices.wd1:R265305' WHERE id = '6d71e617-960e-457b-b445-04ecc35f427e' AND dedup_key = 'rekey-0025:6d71e617-960e-457b-b445-04ecc35f427e';
UPDATE internship_postings SET dedup_key = 'ats:workday:analogdevices.wd1:R265306-1' WHERE id = 'ac409ae3-6bff-4283-b00c-ebb50998fd2e' AND dedup_key = 'rekey-0025:ac409ae3-6bff-4283-b00c-ebb50998fd2e';
UPDATE internship_postings SET dedup_key = 'ats:workday:hitachi.wd1:R0142571' WHERE id = '67c73d70-1223-492d-8ed6-1c8911e33a3b' AND dedup_key = 'rekey-0025:67c73d70-1223-492d-8ed6-1c8911e33a3b';
UPDATE internship_postings SET dedup_key = 'ats:workday:bpinternational.wd3:RQ114655-2' WHERE id = '3b202a9e-30ed-47f5-b661-6616c23bccde' AND dedup_key = 'rekey-0025:3b202a9e-30ed-47f5-b661-6616c23bccde';
UPDATE internship_postings SET dedup_key = 'ats:workday:salesforce.wd12:JR340771-1' WHERE id = '2e9e7357-08e1-44c3-bdb0-94c4168762be' AND dedup_key = 'rekey-0025:2e9e7357-08e1-44c3-bdb0-94c4168762be';
UPDATE internship_postings SET dedup_key = 'ats:workday:nrel.wd5:R14394' WHERE id = '7b80a14e-d35b-48c7-8c9e-fa822bba9647' AND dedup_key = 'rekey-0025:7b80a14e-d35b-48c7-8c9e-fa822bba9647';
UPDATE internship_postings SET dedup_key = 'ats:workday:cccis.wd1:0014827' WHERE id = '7a174162-718f-430a-86b4-84ec0bf041ed' AND dedup_key = 'rekey-0025:7a174162-718f-430a-86b4-84ec0bf041ed';
UPDATE internship_postings SET dedup_key = 'ats:workday:bmo.wd3:R260021769' WHERE id = '3587f771-5997-4b00-8fe7-951450dd675a' AND dedup_key = 'rekey-0025:3587f771-5997-4b00-8fe7-951450dd675a';
UPDATE internship_postings SET dedup_key = 'ats:workday:bah.wd1:R0246869' WHERE id = 'a7b2ac78-0f7c-4fee-b740-6f4efda22cd6' AND dedup_key = 'rekey-0025:a7b2ac78-0f7c-4fee-b740-6f4efda22cd6';
UPDATE internship_postings SET dedup_key = 'ats:workday:mastercard.wd1:R-287618-1' WHERE id = '3687f7f6-8224-4a71-890f-de4bff8e67d7' AND dedup_key = 'rekey-0025:3687f7f6-8224-4a71-890f-de4bff8e67d7';
UPDATE internship_postings SET dedup_key = 'ats:workday:analogdevices.wd1:R265298' WHERE id = '49a71926-8218-4e74-b0ab-e46fe3dd4c6c' AND dedup_key = 'rekey-0025:49a71926-8218-4e74-b0ab-e46fe3dd4c6c';
UPDATE internship_postings SET dedup_key = 'ats:workday:disney.wd5:10158145-1' WHERE id = '2adbf822-bcfa-42fc-af26-4a0cbd96c3f5' AND dedup_key = 'rekey-0025:2adbf822-bcfa-42fc-af26-4a0cbd96c3f5';
UPDATE internship_postings SET dedup_key = 'ats:workday:analogdevices.wd1:R265302' WHERE id = '38770edf-5bcc-4a4e-b83b-72a03f2cb1dc' AND dedup_key = 'rekey-0025:38770edf-5bcc-4a4e-b83b-72a03f2cb1dc';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000185490' WHERE id = 'e0807f40-5698-482e-9be9-35349eced260' AND dedup_key = 'rekey-0025:e0807f40-5698-482e-9be9-35349eced260';
UPDATE internship_postings SET dedup_key = 'ats:workday:motorolasolutions.wd5:R67740' WHERE id = '47f834f8-8fbb-42bc-baf3-9df0cd5d9937' AND dedup_key = 'rekey-0025:47f834f8-8fbb-42bc-baf3-9df0cd5d9937';
UPDATE internship_postings SET dedup_key = 'ats:workday:freddiemac.wd5:JR17544' WHERE id = '6eb2d2f9-8041-4ccd-9db9-6047c967cabc' AND dedup_key = 'rekey-0025:6eb2d2f9-8041-4ccd-9db9-6047c967cabc';
UPDATE internship_postings SET dedup_key = 'ats:workday:freddiemac.wd5:JR17564' WHERE id = '71ac5350-ba2f-479b-acb5-fd38593fd7e0' AND dedup_key = 'rekey-0025:71ac5350-ba2f-479b-acb5-fd38593fd7e0';
UPDATE internship_postings SET dedup_key = 'ats:workday:pg.wd5:R000157499' WHERE id = '2df4a1da-c3e9-419d-a58f-d19ed2ff8a3f' AND dedup_key = 'rekey-0025:2df4a1da-c3e9-419d-a58f-d19ed2ff8a3f';
UPDATE internship_postings SET dedup_key = 'ats:workday:dmainc.wd5:REQ636' WHERE id = '3356814a-8c4b-4c4a-81d2-2845002451dc' AND dedup_key = 'rekey-0025:3356814a-8c4b-4c4a-81d2-2845002451dc';
UPDATE internship_postings SET dedup_key = 'ats:workday:dmainc.wd5:REQ634' WHERE id = '23d72ff2-a484-4806-b407-78d58a74c68a' AND dedup_key = 'rekey-0025:23d72ff2-a484-4806-b407-78d58a74c68a';
UPDATE internship_postings SET dedup_key = 'ats:workday:micron.wd1:JR108977' WHERE id = '67ca9bcc-6257-4834-a19a-499f95e3b134' AND dedup_key = 'rekey-0025:67ca9bcc-6257-4834-a19a-499f95e3b134';
UPDATE internship_postings SET dedup_key = 'ats:workday:monolithicpower.wd12:R-1756' WHERE id = 'a6a256f6-2edd-4d12-9744-7f2664ce4598' AND dedup_key = 'rekey-0025:a6a256f6-2edd-4d12-9744-7f2664ce4598';
UPDATE internship_postings SET dedup_key = 'ats:workday:thehartford.wd5:R2626105-1' WHERE id = '68a42408-ed6b-4628-a155-7836f1f50b04' AND dedup_key = 'rekey-0025:68a42408-ed6b-4628-a155-7836f1f50b04';
UPDATE internship_postings SET dedup_key = 'ats:workday:thehartford.wd5:R2626648' WHERE id = '966a6049-bc05-4757-9999-cdde4d9e16e5' AND dedup_key = 'rekey-0025:966a6049-bc05-4757-9999-cdde4d9e16e5';
UPDATE internship_postings SET dedup_key = 'ats:workday:thehartford.wd5:R2626610' WHERE id = '9791bc0f-443f-4710-b25e-d93f57216b33' AND dedup_key = 'rekey-0025:9791bc0f-443f-4710-b25e-d93f57216b33';
UPDATE internship_postings SET dedup_key = 'ats:workday:brunswick.wd1:JR-051325' WHERE id = '36cd7194-56cb-4ce9-bb42-a780b4c78dd8' AND dedup_key = 'rekey-0025:36cd7194-56cb-4ce9-bb42-a780b4c78dd8';
UPDATE internship_postings SET dedup_key = 'ats:workday:brunswick.wd1:JR-051316' WHERE id = 'a635eb48-3a79-4864-a23b-2f18911f1741' AND dedup_key = 'rekey-0025:a635eb48-3a79-4864-a23b-2f18911f1741';
UPDATE internship_postings SET dedup_key = 'ats:workday:bpinternational.wd3:RQ115146' WHERE id = '1d74cc80-172e-4223-8836-44dad15ef74e' AND dedup_key = 'rekey-0025:1d74cc80-172e-4223-8836-44dad15ef74e';
UPDATE internship_postings SET dedup_key = 'ats:workday:roche.wd3:202608-121800' WHERE id = '1da691f8-d53b-49fe-bf7e-031bf14a5145' AND dedup_key = 'rekey-0025:1da691f8-d53b-49fe-bf7e-031bf14a5145';
UPDATE internship_postings SET dedup_key = 'ats:workday:wexinc.wd5:R22834' WHERE id = '863c2c7c-1c7e-4d02-bc00-44bc176ed8d7' AND dedup_key = 'rekey-0025:863c2c7c-1c7e-4d02-bc00-44bc176ed8d7';
UPDATE internship_postings SET dedup_key = 'ats:workday:thehartford.wd5:R2626609' WHERE id = '7caa91ea-b9a6-4760-b53d-47f5cd4aec75' AND dedup_key = 'rekey-0025:7caa91ea-b9a6-4760-b53d-47f5cd4aec75';
UPDATE internship_postings SET dedup_key = 'ats:workday:haier.wd3:REQ-26592' WHERE id = '23213059-9c79-48bb-9428-8652816ec09e' AND dedup_key = 'rekey-0025:23213059-9c79-48bb-9428-8652816ec09e';
UPDATE internship_postings SET dedup_key = 'ats:workday:haier.wd3:REQ-26596' WHERE id = '2ab2b88d-f5da-4f7c-92ae-1f45951d211a' AND dedup_key = 'rekey-0025:2ab2b88d-f5da-4f7c-92ae-1f45951d211a';
UPDATE internship_postings SET dedup_key = 'ats:workday:brunswick.wd1:JR-051212' WHERE id = '32f8d576-6f4c-4e80-b6aa-601c9196bb7d' AND dedup_key = 'rekey-0025:32f8d576-6f4c-4e80-b6aa-601c9196bb7d';
UPDATE internship_postings SET dedup_key = 'ats:workday:repsol.wd3:83945-1' WHERE id = 'a3080d49-b4ed-4fb8-93ee-69e1dbb1bf3f' AND dedup_key = 'rekey-0025:a3080d49-b4ed-4fb8-93ee-69e1dbb1bf3f';
UPDATE internship_postings SET dedup_key = 'ats:workday:bmo.wd3:R260021769-1' WHERE id = '7bc530ed-7bcd-4c5b-a4d2-fbb24cea880c' AND dedup_key = 'rekey-0025:7bc530ed-7bcd-4c5b-a4d2-fbb24cea880c';
UPDATE internship_postings SET dedup_key = 'ats:workday:parsons.wd5:R185388' WHERE id = 'ba76e785-5847-4294-b060-c07aed97117e' AND dedup_key = 'rekey-0025:ba76e785-5847-4294-b060-c07aed97117e';
UPDATE internship_postings SET dedup_key = 'ats:workday:aoins.wd5:R_12318' WHERE id = 'a39064ae-c0a3-49be-904b-9f0913e8a079' AND dedup_key = 'rekey-0025:a39064ae-c0a3-49be-904b-9f0913e8a079';
UPDATE internship_postings SET dedup_key = 'ats:workday:aoins.wd5:R_14272' WHERE id = '5d8b9b3a-38a5-44fe-a800-a0a96f90d4ac' AND dedup_key = 'rekey-0025:5d8b9b3a-38a5-44fe-a800-a0a96f90d4ac';
UPDATE internship_postings SET dedup_key = 'ats:workday:thehartford.wd5:R2626649' WHERE id = '7c985043-03df-49cf-ab64-68372035049a' AND dedup_key = 'rekey-0025:7c985043-03df-49cf-ab64-68372035049a';
UPDATE internship_postings SET dedup_key = 'ats:workday:ancestry.wd501:R003434' WHERE id = '3feade02-fb9a-421b-bb96-f5e7bcabdd74' AND dedup_key = 'rekey-0025:3feade02-fb9a-421b-bb96-f5e7bcabdd74';
UPDATE internship_postings SET dedup_key = 'ats:workday:caci.wd1:331120' WHERE id = '6efe828b-faa5-43f6-9cf1-a13cc5fe88f4' AND dedup_key = 'rekey-0025:6efe828b-faa5-43f6-9cf1-a13cc5fe88f4';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01866472' WHERE id = '37ce247c-4dab-4422-8592-8ad401305ecd' AND dedup_key = 'rekey-0025:37ce247c-4dab-4422-8592-8ad401305ecd';
UPDATE internship_postings SET dedup_key = 'ats:workday:leidos.wd5:R-00190648' WHERE id = 'e84176ed-1417-43fa-a11b-bd00221afd7a' AND dedup_key = 'rekey-0025:e84176ed-1417-43fa-a11b-bd00221afd7a';
UPDATE internship_postings SET dedup_key = 'ats:workday:repsol.wd3:83947-1' WHERE id = '47fe3df5-c4be-465f-93e9-60eb9352d4d9' AND dedup_key = 'rekey-0025:47fe3df5-c4be-465f-93e9-60eb9352d4d9';
UPDATE internship_postings SET dedup_key = 'ats:workday:repsol.wd3:83951-1' WHERE id = 'a324f366-46da-46f5-9ff9-ccd5ad464b93' AND dedup_key = 'rekey-0025:a324f366-46da-46f5-9ff9-ccd5ad464b93';
UPDATE internship_postings SET dedup_key = 'ats:workday:jj.wd5:R-095741' WHERE id = 'ce601ae2-3489-445c-958d-79d9ece21c60' AND dedup_key = 'rekey-0025:ce601ae2-3489-445c-958d-79d9ece21c60';
UPDATE internship_postings SET dedup_key = 'ats:workday:aoins.wd5:R_2121' WHERE id = '1a5b4a87-3e34-43c3-932d-4664b5295584' AND dedup_key = 'rekey-0025:1a5b4a87-3e34-43c3-932d-4664b5295584';
UPDATE internship_postings SET dedup_key = 'ats:workday:pg.wd5:R000157846' WHERE id = 'fc692b97-6e7d-43ac-8b86-bd74685c8901' AND dedup_key = 'rekey-0025:fc692b97-6e7d-43ac-8b86-bd74685c8901';
UPDATE internship_postings SET dedup_key = 'ats:workday:dimensional.wd5:2026-9025' WHERE id = 'a23c4d72-874d-4ee9-b3c4-36fdb53efac5' AND dedup_key = 'rekey-0025:a23c4d72-874d-4ee9-b3c4-36fdb53efac5';
UPDATE internship_postings SET dedup_key = 'ats:workday:ambarella.wd108:JR100360' WHERE id = '2ede2e43-d02f-4a36-9e87-f563e13f99f6' AND dedup_key = 'rekey-0025:2ede2e43-d02f-4a36-9e87-f563e13f99f6';
UPDATE internship_postings SET dedup_key = 'ats:workday:ambarella.wd108:JR100366-1' WHERE id = 'a26b86e0-0e0a-4cbe-9339-063c90472988' AND dedup_key = 'rekey-0025:a26b86e0-0e0a-4cbe-9339-063c90472988';
UPDATE internship_postings SET dedup_key = 'ats:workday:ambarella.wd108:JR100359' WHERE id = '0230fc8b-327e-4f06-8ebe-d39bc486e75d' AND dedup_key = 'rekey-0025:0230fc8b-327e-4f06-8ebe-d39bc486e75d';
UPDATE internship_postings SET dedup_key = 'ats:workday:ambarella.wd108:JR100365' WHERE id = 'e2559a75-d614-4170-a80b-366b38e0083a' AND dedup_key = 'rekey-0025:e2559a75-d614-4170-a80b-366b38e0083a';
UPDATE internship_postings SET dedup_key = 'ats:workday:bah.wd1:R0248403' WHERE id = 'c9037d61-5931-440e-9f1f-ed31deaa84bb' AND dedup_key = 'rekey-0025:c9037d61-5931-440e-9f1f-ed31deaa84bb';
UPDATE internship_postings SET dedup_key = 'ats:workday:bah.wd1:R0248141' WHERE id = '6a70d810-8b81-45d4-aa5b-4642ab420ade' AND dedup_key = 'rekey-0025:6a70d810-8b81-45d4-aa5b-4642ab420ade';
UPDATE internship_postings SET dedup_key = 'ats:workday:leidos.wd5:R-00190756' WHERE id = '23644019-1e80-4a25-b864-c6437fe07924' AND dedup_key = 'rekey-0025:23644019-1e80-4a25-b864-c6437fe07924';
UPDATE internship_postings SET dedup_key = 'ats:workday:leidos.wd5:R-00190766' WHERE id = '390a0c19-3b06-4107-97d1-6ed7bbe5b826' AND dedup_key = 'rekey-0025:390a0c19-3b06-4107-97d1-6ed7bbe5b826';
UPDATE internship_postings SET dedup_key = 'ats:workday:manulife.wd3:JR26081684' WHERE id = '08c41772-0de1-4d3b-a468-a1336f0e6f50' AND dedup_key = 'rekey-0025:08c41772-0de1-4d3b-a468-a1336f0e6f50';
UPDATE internship_postings SET dedup_key = 'ats:workday:microchiphr.wd5:R3714-26' WHERE id = 'ff06c66d-92f9-4c1c-a2f7-d2efe238b872' AND dedup_key = 'rekey-0025:ff06c66d-92f9-4c1c-a2f7-d2efe238b872';
UPDATE internship_postings SET dedup_key = 'ats:workday:medtronic.wd1:R76021' WHERE id = '0b2db226-9db9-4aad-b4d2-93f661a6ca6f' AND dedup_key = 'rekey-0025:0b2db226-9db9-4aad-b4d2-93f661a6ca6f';
UPDATE internship_postings SET dedup_key = 'ats:workday:finastra.wd3:REQ0826_0038079' WHERE id = '0606e4aa-90e1-4ba4-9873-1fb9547b2ab5' AND dedup_key = 'rekey-0025:0606e4aa-90e1-4ba4-9873-1fb9547b2ab5';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000185825' WHERE id = '7f6b4775-dd78-4e51-9fcc-8397753ccbd6' AND dedup_key = 'rekey-0025:7f6b4775-dd78-4e51-9fcc-8397753ccbd6';
UPDATE internship_postings SET dedup_key = 'ats:workday:rbc.wd3:R-0000185825-1' WHERE id = 'ba6d2323-8f60-4198-bfe5-568a761cc052' AND dedup_key = 'rekey-0025:ba6d2323-8f60-4198-bfe5-568a761cc052';
UPDATE internship_postings SET dedup_key = 'ats:workday:leidos.wd5:R-00190672-1' WHERE id = 'c7efcf06-fae2-4ff7-873b-3a029a5db957' AND dedup_key = 'rekey-0025:c7efcf06-fae2-4ff7-873b-3a029a5db957';
UPDATE internship_postings SET dedup_key = 'ats:workday:brunswick.wd1:JR-051312' WHERE id = '26464b7b-e542-486c-8d00-5fd59274b795' AND dedup_key = 'rekey-0025:26464b7b-e542-486c-8d00-5fd59274b795';
UPDATE internship_postings SET dedup_key = 'ats:workday:analogdevices.wd1:R265579' WHERE id = 'cea1a3a5-c2a1-44bb-b0cf-a86f42c0d978' AND dedup_key = 'rekey-0025:cea1a3a5-c2a1-44bb-b0cf-a86f42c0d978';
UPDATE internship_postings SET dedup_key = 'ats:workday:motorolasolutions.wd5:R67782' WHERE id = 'c0178f45-4188-40d0-9aae-3348239e31cd' AND dedup_key = 'rekey-0025:c0178f45-4188-40d0-9aae-3348239e31cd';
UPDATE internship_postings SET dedup_key = 'ats:workday:mastercard.wd1:R-284901' WHERE id = '311c3e25-9ea9-4584-a06b-ebc6e482ec23' AND dedup_key = 'rekey-0025:311c3e25-9ea9-4584-a06b-ebc6e482ec23';
UPDATE internship_postings SET dedup_key = 'ats:workday:bah.wd1:R0248115' WHERE id = '59ca5337-2595-4774-b2b4-cfaec3b8bfab' AND dedup_key = 'rekey-0025:59ca5337-2595-4774-b2b4-cfaec3b8bfab';
UPDATE internship_postings SET dedup_key = 'ats:workday:bah.wd1:R0248130' WHERE id = '65983bc1-74a9-46e0-925d-e2266c8ea4a6' AND dedup_key = 'rekey-0025:65983bc1-74a9-46e0-925d-e2266c8ea4a6';
UPDATE internship_postings SET dedup_key = 'ats:workday:disney.wd5:10158599' WHERE id = '48481e0d-31e2-49bf-9a15-5cb4dd540da4' AND dedup_key = 'rekey-0025:48481e0d-31e2-49bf-9a15-5cb4dd540da4';
UPDATE internship_postings SET dedup_key = 'ats:workday:brunswick.wd1:JR-051426-1' WHERE id = '6aa58a11-ff84-49e9-853a-3313df8a50f7' AND dedup_key = 'rekey-0025:6aa58a11-ff84-49e9-853a-3313df8a50f7';
UPDATE internship_postings SET dedup_key = 'ats:workday:selinc.wd1:2025-18137' WHERE id = 'a90cfbaa-1d75-42ea-a011-54ee775208be' AND dedup_key = 'rekey-0025:a90cfbaa-1d75-42ea-a011-54ee775208be';
UPDATE internship_postings SET dedup_key = 'ats:workday:philips.wd3:590404' WHERE id = 'c8e47d6f-7514-4528-a467-62455f03cda6' AND dedup_key = 'rekey-0025:c8e47d6f-7514-4528-a467-62455f03cda6';
UPDATE internship_postings SET dedup_key = 'ats:workday:cadence.wd1:R56108-2' WHERE id = 'f338c11e-9944-4169-898f-f811a6b89771' AND dedup_key = 'rekey-0025:f338c11e-9944-4169-898f-f811a6b89771';
UPDATE internship_postings SET dedup_key = 'ats:workday:equifax.wd5:J00178784' WHERE id = '583e679e-b183-47b6-bac8-9c36a3100c35' AND dedup_key = 'rekey-0025:583e679e-b183-47b6-bac8-9c36a3100c35';
UPDATE internship_postings SET dedup_key = 'ats:workday:intelcomgroup.wd3:JR111570-1' WHERE id = 'a7ce2834-e8de-463e-a075-256d5a3dacce' AND dedup_key = 'rekey-0025:a7ce2834-e8de-463e-a075-256d5a3dacce';
UPDATE internship_postings SET dedup_key = 'ats:workday:intelcomgroup.wd3:JR111571' WHERE id = '88404599-3905-40f8-849c-fd58c68633e2' AND dedup_key = 'rekey-0025:88404599-3905-40f8-849c-fd58c68633e2';
UPDATE internship_postings SET dedup_key = 'ats:workday:intelcomgroup.wd3:JR111555' WHERE id = 'f5074b11-9079-4181-a60e-e57e64db97ad' AND dedup_key = 'rekey-0025:f5074b11-9079-4181-a60e-e57e64db97ad';
UPDATE internship_postings SET dedup_key = 'ats:workday:intelcomgroup.wd3:JR111567' WHERE id = '66ad12e8-81bc-4db2-9394-1c852c2bb6e1' AND dedup_key = 'rekey-0025:66ad12e8-81bc-4db2-9394-1c852c2bb6e1';
UPDATE internship_postings SET dedup_key = 'ats:workday:geaerospace.wd5:R5039041-1' WHERE id = '98b154d4-7cff-4d1a-8052-3b1a43d5230a' AND dedup_key = 'rekey-0025:98b154d4-7cff-4d1a-8052-3b1a43d5230a';
UPDATE internship_postings SET dedup_key = 'ats:workday:stryker.wd1:R572632-1' WHERE id = '56093405-e5ef-4a46-aeb9-a9078e4b3de6' AND dedup_key = 'rekey-0025:56093405-e5ef-4a46-aeb9-a9078e4b3de6';
UPDATE internship_postings SET dedup_key = 'ats:workday:stryker.wd1:R572624' WHERE id = '31397148-26b5-4889-aa7b-7dc01294af6b' AND dedup_key = 'rekey-0025:31397148-26b5-4889-aa7b-7dc01294af6b';
UPDATE internship_postings SET dedup_key = 'ats:workday:copart.wd12:JR110948' WHERE id = 'cc176323-4119-4148-a588-fdce1180fff8' AND dedup_key = 'rekey-0025:cc176323-4119-4148-a588-fdce1180fff8';
UPDATE internship_postings SET dedup_key = 'ats:workday:homedepot.wd5:Req191937' WHERE id = 'ccec06a0-c7a8-4bf6-b7ad-4ab1ee91d2b3' AND dedup_key = 'rekey-0025:ccec06a0-c7a8-4bf6-b7ad-4ab1ee91d2b3';
UPDATE internship_postings SET dedup_key = 'ats:workday:medline.wd5:R2617378' WHERE id = '2543dbcf-4d1f-4241-b730-09aca6e1fb40' AND dedup_key = 'rekey-0025:2543dbcf-4d1f-4241-b730-09aca6e1fb40';
UPDATE internship_postings SET dedup_key = 'ats:workday:draftkings.wd1:JR14929' WHERE id = 'd5f46fe1-4a73-44ea-8fb7-b99accddb887' AND dedup_key = 'rekey-0025:d5f46fe1-4a73-44ea-8fb7-b99accddb887';
UPDATE internship_postings SET dedup_key = 'ats:workday:draftkings.wd1:JR14928' WHERE id = '77626970-4241-4468-8226-9552bc9d2a19' AND dedup_key = 'rekey-0025:77626970-4241-4468-8226-9552bc9d2a19';
UPDATE internship_postings SET dedup_key = 'ats:workday:nisource.wd1:R00943449' WHERE id = '121ea21f-5b17-44e4-aa18-e424b748fea7' AND dedup_key = 'rekey-0025:121ea21f-5b17-44e4-aa18-e424b748fea7';
UPDATE internship_postings SET dedup_key = 'ats:workday:oshkoshcorporation.wd5:R49786' WHERE id = 'e3728655-5e92-4fdb-9b4b-ced32dbfbeb1' AND dedup_key = 'rekey-0025:e3728655-5e92-4fdb-9b4b-ced32dbfbeb1';
UPDATE internship_postings SET dedup_key = 'ats:workday:cibc.wd3:2617782' WHERE id = '01f5f170-f8e9-4388-9ffb-70622fe4d329' AND dedup_key = 'rekey-0025:01f5f170-f8e9-4388-9ffb-70622fe4d329';
UPDATE internship_postings SET dedup_key = 'ats:workday:manulife.wd3:JR26081661' WHERE id = 'd0b4a134-2a08-4f04-98cb-455ff2d6a6b9' AND dedup_key = 'rekey-0025:d0b4a134-2a08-4f04-98cb-455ff2d6a6b9';
UPDATE internship_postings SET dedup_key = 'ats:workday:manulife.wd3:JR26081658' WHERE id = 'fb6736ea-1204-47a3-96e2-54eb846705e3' AND dedup_key = 'rekey-0025:fb6736ea-1204-47a3-96e2-54eb846705e3';
UPDATE internship_postings SET dedup_key = 'ats:workday:manulife.wd3:JR26081685' WHERE id = '662d2eb3-1526-407c-bcc0-40bd2181dcaa' AND dedup_key = 'rekey-0025:662d2eb3-1526-407c-bcc0-40bd2181dcaa';
UPDATE internship_postings SET dedup_key = 'ats:workday:generalmotors.wd5:JR-202618353' WHERE id = '45476c70-3868-4287-b1a5-089ceeeefaa2' AND dedup_key = 'rekey-0025:45476c70-3868-4287-b1a5-089ceeeefaa2';
UPDATE internship_postings SET dedup_key = 'ats:workday:nike.wd1:R-91110' WHERE id = '23d5df93-9637-4c04-a2c4-aa73bbb76c6c' AND dedup_key = 'rekey-0025:23d5df93-9637-4c04-a2c4-aa73bbb76c6c';
UPDATE internship_postings SET dedup_key = 'ats:workday:geaerospace.wd5:R5039185-1' WHERE id = 'c799d6e7-bbda-4c4a-b65b-88847e70243f' AND dedup_key = 'rekey-0025:c799d6e7-bbda-4c4a-b65b-88847e70243f';
UPDATE internship_postings SET dedup_key = 'ats:workday:nike.wd1:R-91111' WHERE id = '025f5632-522a-4c2a-8600-9cc4895baa89' AND dedup_key = 'rekey-0025:025f5632-522a-4c2a-8600-9cc4895baa89';
UPDATE internship_postings SET dedup_key = 'ats:workday:pae.wd1:R0169322' WHERE id = '19db709b-3f6e-497e-9c47-f0bde2c42405' AND dedup_key = 'rekey-0025:19db709b-3f6e-497e-9c47-f0bde2c42405';
UPDATE internship_postings SET dedup_key = 'ats:workday:caci.wd1:331354' WHERE id = '3d025d6e-9284-424b-b82e-7d2f64a9874b' AND dedup_key = 'rekey-0025:3d025d6e-9284-424b-b82e-7d2f64a9874b';
UPDATE internship_postings SET dedup_key = 'ats:workday:caci.wd1:331359' WHERE id = 'bab1dade-4df9-425f-ade5-e6c4179df220' AND dedup_key = 'rekey-0025:bab1dade-4df9-425f-ade5-e6c4179df220';
UPDATE internship_postings SET dedup_key = 'ats:workday:caci.wd1:331356-1' WHERE id = 'b597d520-9b61-4cdd-8c6e-c07f8147259c' AND dedup_key = 'rekey-0025:b597d520-9b61-4cdd-8c6e-c07f8147259c';
UPDATE internship_postings SET dedup_key = 'ats:workday:gehc.wd5:R4043933-2' WHERE id = '2cf0f26e-090d-43bc-b123-533f38de0db1' AND dedup_key = 'rekey-0025:2cf0f26e-090d-43bc-b123-533f38de0db1';
UPDATE internship_postings SET dedup_key = 'ats:workday:micron.wd1:JR109990' WHERE id = '3f7fd285-4d1e-40a9-8bc8-09b107ee933e' AND dedup_key = 'rekey-0025:3f7fd285-4d1e-40a9-8bc8-09b107ee933e';
UPDATE internship_postings SET dedup_key = 'ats:workday:draper.wd5:JR002884' WHERE id = '1efcf523-c326-4bbc-b131-ece3c213387f' AND dedup_key = 'rekey-0025:1efcf523-c326-4bbc-b131-ece3c213387f';
UPDATE internship_postings SET dedup_key = 'ats:workday:igsenergy.wd1:R6263' WHERE id = 'd989c117-0da2-4e11-9400-3b346a18f9af' AND dedup_key = 'rekey-0025:d989c117-0da2-4e11-9400-3b346a18f9af';
UPDATE internship_postings SET dedup_key = 'ats:workday:brunswick.wd1:JR-051436' WHERE id = 'a6ed9765-78d4-4c7a-859a-5da3e0ea24da' AND dedup_key = 'rekey-0025:a6ed9765-78d4-4c7a-859a-5da3e0ea24da';
UPDATE internship_postings SET dedup_key = 'ats:workday:philips.wd3:590901' WHERE id = '74f3b561-04c3-4b0d-82fd-1036a70ff453' AND dedup_key = 'rekey-0025:74f3b561-04c3-4b0d-82fd-1036a70ff453';
UPDATE internship_postings SET dedup_key = 'ats:workday:adobe.wd5:R171519' WHERE id = '18cbde3f-b37b-4192-b643-e5dd9cc45ccc' AND dedup_key = 'rekey-0025:18cbde3f-b37b-4192-b643-e5dd9cc45ccc';
UPDATE internship_postings SET dedup_key = 'ats:workday:travelers.wd5:R-52270' WHERE id = 'a1b1f44f-6b66-4910-a655-9c546934a260' AND dedup_key = 'rekey-0025:a1b1f44f-6b66-4910-a655-9c546934a260';
UPDATE internship_postings SET dedup_key = 'ats:workday:tcenergy.wd3:JR-10741' WHERE id = '54315c65-6b92-420b-be9a-4b743ed61c0f' AND dedup_key = 'rekey-0025:54315c65-6b92-420b-be9a-4b743ed61c0f';
UPDATE internship_postings SET dedup_key = 'ats:workday:swa.wd1:R-2026-71386' WHERE id = '44f3e4ca-586b-4d52-a86e-bf010b611201' AND dedup_key = 'rekey-0025:44f3e4ca-586b-4d52-a86e-bf010b611201';
UPDATE internship_postings SET dedup_key = 'ats:workday:fifththird.wd5:R71587' WHERE id = 'd9028163-b2cf-48b7-9d6f-efccf8c10d51' AND dedup_key = 'rekey-0025:d9028163-b2cf-48b7-9d6f-efccf8c10d51';
UPDATE internship_postings SET dedup_key = 'ats:workday:tcenergy.wd3:JR-10742' WHERE id = 'e46962fa-af9f-41c1-86df-53943d926482' AND dedup_key = 'rekey-0025:e46962fa-af9f-41c1-86df-53943d926482';
UPDATE internship_postings SET dedup_key = 'ats:workday:vermeer.wd5:REQ-22165' WHERE id = '9081172b-7f90-4952-acdb-9bf1f21f935e' AND dedup_key = 'rekey-0025:9081172b-7f90-4952-acdb-9bf1f21f935e';
UPDATE internship_postings SET dedup_key = 'ats:workday:tcenergy.wd3:JR-10728' WHERE id = '3accbec6-ed81-4459-93f0-29c3ec261cb5' AND dedup_key = 'rekey-0025:3accbec6-ed81-4459-93f0-29c3ec261cb5';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01871187' WHERE id = 'df44b4a1-da53-4f7c-851e-3aa5a343ac4b' AND dedup_key = 'rekey-0025:df44b4a1-da53-4f7c-851e-3aa5a343ac4b';
UPDATE internship_postings SET dedup_key = 'ats:workday:caci.wd1:331393' WHERE id = '2948e2a6-15fd-40f5-a331-51f16e95fbd6' AND dedup_key = 'rekey-0025:2948e2a6-15fd-40f5-a331-51f16e95fbd6';
UPDATE internship_postings SET dedup_key = 'ats:workday:caci.wd1:331368' WHERE id = 'a9c9d116-df26-4efb-8ecc-df2cb57e25de' AND dedup_key = 'rekey-0025:a9c9d116-df26-4efb-8ecc-df2cb57e25de';
UPDATE internship_postings SET dedup_key = 'ats:workday:magna.wd3:R00259672' WHERE id = 'c02bacf0-7540-499b-92af-6b976f641154' AND dedup_key = 'rekey-0025:c02bacf0-7540-499b-92af-6b976f641154';
UPDATE internship_postings SET dedup_key = 'ats:workday:nisource.wd1:R00943339' WHERE id = '9814475d-a99e-4dea-8547-6963d1cc7287' AND dedup_key = 'rekey-0025:9814475d-a99e-4dea-8547-6963d1cc7287';
UPDATE internship_postings SET dedup_key = 'ats:workday:psu.wd1:REQ_0000082190-2' WHERE id = '88c8c512-735d-46e3-82c2-d14a81d6bb17' AND dedup_key = 'rekey-0025:88c8c512-735d-46e3-82c2-d14a81d6bb17';
UPDATE internship_postings SET dedup_key = 'ats:workday:vermeer.wd5:REQ-22163' WHERE id = '41adb144-1152-45f8-a05e-c763c77c2d83' AND dedup_key = 'rekey-0025:41adb144-1152-45f8-a05e-c763c77c2d83';
UPDATE internship_postings SET dedup_key = 'ats:workday:bah.wd1:R0248361' WHERE id = 'ee9a7a32-6a72-40c3-9cd4-4bdcea465d1f' AND dedup_key = 'rekey-0025:ee9a7a32-6a72-40c3-9cd4-4bdcea465d1f';
UPDATE internship_postings SET dedup_key = 'ats:workday:bah.wd1:R0248381' WHERE id = '84345318-a996-494b-aeb4-b60b5e4f8700' AND dedup_key = 'rekey-0025:84345318-a996-494b-aeb4-b60b5e4f8700';
UPDATE internship_postings SET dedup_key = 'ats:workday:bah.wd1:R0248386' WHERE id = 'f2e2ed4f-5e65-402d-b35d-f035473d3e2a' AND dedup_key = 'rekey-0025:f2e2ed4f-5e65-402d-b35d-f035473d3e2a';
UPDATE internship_postings SET dedup_key = 'ats:workday:trumpf.wd3:R00042681' WHERE id = 'df42698d-83a3-4d48-8874-f455131cbb77' AND dedup_key = 'rekey-0025:df42698d-83a3-4d48-8874-f455131cbb77';
UPDATE internship_postings SET dedup_key = 'ats:workday:sbdinc.wd1:REQ-1000052019' WHERE id = '68c703f9-a8a1-4d98-913f-9119ac1563e5' AND dedup_key = 'rekey-0025:68c703f9-a8a1-4d98-913f-9119ac1563e5';
UPDATE internship_postings SET dedup_key = 'ats:workday:flir.wd1:REQ36667' WHERE id = '2e92e618-7647-41f1-8549-f1f260f740c2' AND dedup_key = 'rekey-0025:2e92e618-7647-41f1-8549-f1f260f740c2';
UPDATE internship_postings SET dedup_key = 'ats:workday:micron.wd1:JR109583' WHERE id = '20e68190-6587-479c-87c4-84258b308b91' AND dedup_key = 'rekey-0025:20e68190-6587-479c-87c4-84258b308b91';
UPDATE internship_postings SET dedup_key = 'ats:workday:medline.wd5:R2617613' WHERE id = '26d037ae-b926-4f3b-9626-81c94acd3810' AND dedup_key = 'rekey-0025:26d037ae-b926-4f3b-9626-81c94acd3810';
UPDATE internship_postings SET dedup_key = 'ats:workday:blueorigin.wd5:R71434' WHERE id = 'cccfec40-58a9-4258-b6c0-2fe89a3080d5' AND dedup_key = 'rekey-0025:cccfec40-58a9-4258-b6c0-2fe89a3080d5';
UPDATE internship_postings SET dedup_key = 'ats:workday:blueorigin.wd5:R71423' WHERE id = '306f4cca-6a8b-485f-bf68-accda485e779' AND dedup_key = 'rekey-0025:306f4cca-6a8b-485f-bf68-accda485e779';
UPDATE internship_postings SET dedup_key = 'ats:workday:blueorigin.wd5:R71424' WHERE id = '2f03f5ca-6755-42e6-abbc-3bee690f16fa' AND dedup_key = 'rekey-0025:2f03f5ca-6755-42e6-abbc-3bee690f16fa';
UPDATE internship_postings SET dedup_key = 'ats:workday:blueorigin.wd5:R71432' WHERE id = '05af2008-3382-40a5-8614-444a71054727' AND dedup_key = 'rekey-0025:05af2008-3382-40a5-8614-444a71054727';
UPDATE internship_postings SET dedup_key = 'ats:workday:blueorigin.wd5:R71425' WHERE id = '6221c6d7-597b-41e1-84b6-bbd45b977082' AND dedup_key = 'rekey-0025:6221c6d7-597b-41e1-84b6-bbd45b977082';
UPDATE internship_postings SET dedup_key = 'ats:workday:philips.wd3:588891' WHERE id = '1844d494-1060-4f24-9af2-644def72f9e5' AND dedup_key = 'rekey-0025:1844d494-1060-4f24-9af2-644def72f9e5';
UPDATE internship_postings SET dedup_key = 'ats:workday:bah.wd1:R0248404' WHERE id = 'c04d5f4c-460c-4430-a563-a95bdb8a8b91' AND dedup_key = 'rekey-0025:c04d5f4c-460c-4430-a563-a95bdb8a8b91';
UPDATE internship_postings SET dedup_key = 'ats:workday:philips.wd3:590093' WHERE id = '038eb441-ccf4-455e-8f4f-d75b20b375a7' AND dedup_key = 'rekey-0025:038eb441-ccf4-455e-8f4f-d75b20b375a7';
UPDATE internship_postings SET dedup_key = 'ats:workday:philips.wd3:587484' WHERE id = '60399bb1-d3fe-4f39-88e6-f532ec6dcfc7' AND dedup_key = 'rekey-0025:60399bb1-d3fe-4f39-88e6-f532ec6dcfc7';
UPDATE internship_postings SET dedup_key = 'ats:workday:philips.wd3:590097' WHERE id = '288d2881-a72e-4ba5-950c-58b151eddf43' AND dedup_key = 'rekey-0025:288d2881-a72e-4ba5-950c-58b151eddf43';
UPDATE internship_postings SET dedup_key = 'ats:workday:philips.wd3:587486' WHERE id = '70779e40-d3aa-4ee9-8671-96b6bc3498a5' AND dedup_key = 'rekey-0025:70779e40-d3aa-4ee9-8671-96b6bc3498a5';
UPDATE internship_postings SET dedup_key = 'ats:workday:jj.wd5:R-096743' WHERE id = 'd57d9a15-0103-4de8-8b78-b39006708b0e' AND dedup_key = 'rekey-0025:d57d9a15-0103-4de8-8b78-b39006708b0e';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01866027' WHERE id = 'f0aefc69-5099-4155-93f3-1ec4fd173bf2' AND dedup_key = 'rekey-0025:f0aefc69-5099-4155-93f3-1ec4fd173bf2';
UPDATE internship_postings SET dedup_key = 'ats:workday:intelcomgroup.wd3:JR111611' WHERE id = 'dfd29b25-32f6-4ca6-8d39-92a9eb6e8bae' AND dedup_key = 'rekey-0025:dfd29b25-32f6-4ca6-8d39-92a9eb6e8bae';
UPDATE internship_postings SET dedup_key = 'ats:workday:newrez.wd1:R10390' WHERE id = '9ab9318a-4450-4e85-8a1e-6a404d99baca' AND dedup_key = 'rekey-0025:9ab9318a-4450-4e85-8a1e-6a404d99baca';
UPDATE internship_postings SET dedup_key = 'ats:workday:clarios.wd5:WD49962' WHERE id = '4ca35f82-dfe4-4668-a7b9-8a3cdfad6ecb' AND dedup_key = 'rekey-0025:4ca35f82-dfe4-4668-a7b9-8a3cdfad6ecb';
UPDATE internship_postings SET dedup_key = 'ats:workday:geico.wd1:R0065435' WHERE id = 'ab3be926-a244-41e6-bd75-9ec56c018652' AND dedup_key = 'rekey-0025:ab3be926-a244-41e6-bd75-9ec56c018652';
UPDATE internship_postings SET dedup_key = 'ats:workday:nike.wd1:R-91228' WHERE id = '5c1b34de-6647-4da1-b480-228b1afb91d4' AND dedup_key = 'rekey-0025:5c1b34de-6647-4da1-b480-228b1afb91d4';
UPDATE internship_postings SET dedup_key = 'ats:workday:clearwateranalytics.wd1:R12215' WHERE id = 'ebbbff0e-417e-492b-8f9a-d6dfe368bff3' AND dedup_key = 'rekey-0025:ebbbff0e-417e-492b-8f9a-d6dfe368bff3';
UPDATE internship_postings SET dedup_key = 'ats:workable:twgai:772CD136FF' WHERE id = '7f2a6f9d-fb7d-47e7-8e95-c7118ba9e41f' AND dedup_key = 'rekey-0025:7f2a6f9d-fb7d-47e7-8e95-c7118ba9e41f';
UPDATE internship_postings SET dedup_key = 'ats:workable:onlogic-inc:10EC1527D8' WHERE id = 'ba6661c7-f32e-4967-931c-1f0bfacdfaaf' AND dedup_key = 'rekey-0025:ba6661c7-f32e-4967-931c-1f0bfacdfaaf';
UPDATE internship_postings SET dedup_key = 'ats:workday:genpt.wd1:R26_0000029135' WHERE id = '832bdf3d-3de6-4cc1-b5a8-423f539fcbab' AND dedup_key = 'rekey-0025:832bdf3d-3de6-4cc1-b5a8-423f539fcbab';
UPDATE internship_postings SET dedup_key = 'ats:workday:genpt.wd1:R26_0000029140' WHERE id = '58382d6f-b6e6-45c7-84a9-cd06c10c5c3a' AND dedup_key = 'rekey-0025:58382d6f-b6e6-45c7-84a9-cd06c10c5c3a';
UPDATE internship_postings SET dedup_key = 'ats:workday:mcgill.wd3:JR0000079762' WHERE id = '202edd56-98fd-4246-9fec-39e8e11e5e5a' AND dedup_key = 'rekey-0025:202edd56-98fd-4246-9fec-39e8e11e5e5a';
UPDATE internship_postings SET dedup_key = 'ats:workday:parsons.wd5:R185565' WHERE id = '40010250-44f4-40b0-aa71-8a26c6649cbb' AND dedup_key = 'rekey-0025:40010250-44f4-40b0-aa71-8a26c6649cbb';
UPDATE internship_postings SET dedup_key = 'ats:workday:medline.wd5:R2617623' WHERE id = 'a06b093e-cb4b-4a09-8c85-1f135b67e27c' AND dedup_key = 'rekey-0025:a06b093e-cb4b-4a09-8c85-1f135b67e27c';
UPDATE internship_postings SET dedup_key = 'ats:workday:usfoods.wd1:R282111' WHERE id = '9d5719c5-df03-4650-b38e-a778333fb9d1' AND dedup_key = 'rekey-0025:9d5719c5-df03-4650-b38e-a778333fb9d1';
UPDATE internship_postings SET dedup_key = 'ats:workday:intelcomgroup.wd3:JR111615-1' WHERE id = 'c4b715dc-1da8-4461-854c-e74c7386b620' AND dedup_key = 'rekey-0025:c4b715dc-1da8-4461-854c-e74c7386b620';
UPDATE internship_postings SET dedup_key = 'ats:workday:usfoods.wd1:R282106' WHERE id = 'f321a663-35f0-49a4-b559-2b2e7848acf4' AND dedup_key = 'rekey-0025:f321a663-35f0-49a4-b559-2b2e7848acf4';
UPDATE internship_postings SET dedup_key = 'ats:workday:poet.wd1:R101679' WHERE id = 'f268456c-0cba-45f8-aa73-d83fbf3126ea' AND dedup_key = 'rekey-0025:f268456c-0cba-45f8-aa73-d83fbf3126ea';
UPDATE internship_postings SET dedup_key = 'ats:workday:usfoods.wd1:R282116' WHERE id = 'c7301677-c865-490a-9f28-86c40971a00d' AND dedup_key = 'rekey-0025:c7301677-c865-490a-9f28-86c40971a00d';
UPDATE internship_postings SET dedup_key = 'ats:workday:fnbcorp.wd501:2026-01851' WHERE id = '99a33ef9-e855-49af-a119-483cc6c0a10e' AND dedup_key = 'rekey-0025:99a33ef9-e855-49af-a119-483cc6c0a10e';
UPDATE internship_postings SET dedup_key = 'ats:workday:fnbcorp.wd501:2026-01714' WHERE id = '2c36b39f-0abc-4122-929a-6cfe210f3fa4' AND dedup_key = 'rekey-0025:2c36b39f-0abc-4122-929a-6cfe210f3fa4';
UPDATE internship_postings SET dedup_key = 'ats:workday:fnbcorp.wd501:2026-01712' WHERE id = 'bd21a75c-d5c1-4db4-ad17-347badb0036d' AND dedup_key = 'rekey-0025:bd21a75c-d5c1-4db4-ad17-347badb0036d';
UPDATE internship_postings SET dedup_key = 'ats:workday:avav.wd1:8388' WHERE id = 'c85e296e-57c6-4ad3-886d-9e25013c6698' AND dedup_key = 'rekey-0025:c85e296e-57c6-4ad3-886d-9e25013c6698';
UPDATE internship_postings SET dedup_key = 'ats:workday:avav.wd1:8389' WHERE id = '3b84ee5d-1965-4aa9-8637-23dc22cba0b0' AND dedup_key = 'rekey-0025:3b84ee5d-1965-4aa9-8637-23dc22cba0b0';
UPDATE internship_postings SET dedup_key = 'ats:workday:avav.wd1:8589' WHERE id = 'c83a38b3-5feb-4657-a5ac-22d710bf30d5' AND dedup_key = 'rekey-0025:c83a38b3-5feb-4657-a5ac-22d710bf30d5';
UPDATE internship_postings SET dedup_key = 'ats:workday:avav.wd1:8611' WHERE id = '927bda0d-9e2d-492d-a33b-04c0730fa237' AND dedup_key = 'rekey-0025:927bda0d-9e2d-492d-a33b-04c0730fa237';
UPDATE internship_postings SET dedup_key = 'ats:workday:vermeer.wd5:REQ-22178' WHERE id = '240eebea-de04-4704-84d6-ce3ef14b2990' AND dedup_key = 'rekey-0025:240eebea-de04-4704-84d6-ce3ef14b2990';
UPDATE internship_postings SET dedup_key = 'ats:workday:rockwellautomation.wd1:R26-5010-1' WHERE id = 'bc30a8d9-f439-4a8c-aeaf-21127da962bd' AND dedup_key = 'rekey-0025:bc30a8d9-f439-4a8c-aeaf-21127da962bd';
UPDATE internship_postings SET dedup_key = 'ats:workday:ntrs.wd1:R160832-1' WHERE id = 'a055bf39-d4af-4632-909c-83d79b9fe497' AND dedup_key = 'rekey-0025:a055bf39-d4af-4632-909c-83d79b9fe497';
UPDATE internship_postings SET dedup_key = 'ats:workday:clearwateranalytics.wd1:R12182' WHERE id = '7ce64850-5a29-48ca-adac-b8f614e0ef2b' AND dedup_key = 'rekey-0025:7ce64850-5a29-48ca-adac-b8f614e0ef2b';
UPDATE internship_postings SET dedup_key = 'ats:workday:clearwateranalytics.wd1:R12192' WHERE id = 'dcd42dfa-c0f1-435e-a24e-75a25b8e6880' AND dedup_key = 'rekey-0025:dcd42dfa-c0f1-435e-a24e-75a25b8e6880';
UPDATE internship_postings SET dedup_key = 'ats:workday:clearwateranalytics.wd1:R12058' WHERE id = '7e6ad594-96d0-40a2-8355-eba62086a3e5' AND dedup_key = 'rekey-0025:7e6ad594-96d0-40a2-8355-eba62086a3e5';
UPDATE internship_postings SET dedup_key = 'ats:workday:michelinhr.wd3:R-2026030979' WHERE id = 'f54a2c64-cbf9-42b6-9e4d-c9cf1d6d08f5' AND dedup_key = 'rekey-0025:f54a2c64-cbf9-42b6-9e4d-c9cf1d6d08f5';
UPDATE internship_postings SET dedup_key = 'ats:workday:globalhr.wd5:01868700' WHERE id = '4721be6e-7e99-4fd2-820d-237c8192d5fb' AND dedup_key = 'rekey-0025:4721be6e-7e99-4fd2-820d-237c8192d5fb';
UPDATE internship_postings SET dedup_key = 'ats:workday:genpt.wd1:R26_0000029238' WHERE id = '8ced0d9e-a6e8-421a-bc0f-50158debc94d' AND dedup_key = 'rekey-0025:8ced0d9e-a6e8-421a-bc0f-50158debc94d';
UPDATE internship_postings SET dedup_key = 'ats:workday:genpt.wd1:R26_0000029235' WHERE id = 'ee4f664e-4994-4c3c-a688-7ebcd72ffe1b' AND dedup_key = 'rekey-0025:ee4f664e-4994-4c3c-a688-7ebcd72ffe1b';
UPDATE internship_postings SET dedup_key = 'ats:workday:genpt.wd1:R26_0000029236' WHERE id = '37ca8fd0-05c0-43de-8c2f-f34a5462b955' AND dedup_key = 'rekey-0025:37ca8fd0-05c0-43de-8c2f-f34a5462b955';
