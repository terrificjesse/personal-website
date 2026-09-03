-- 0028 — give the Greenhouse sightings that predate 0026 the scope tag they would have got.
--
-- GENERATED. Do not hand-edit; regenerate and re-review instead:
--
--     sqlite3 fridge.db ".backup '/tmp/scope-backfill.db'"
--     SCOPE_FIXTURE_DB=/tmp/scope-backfill.db SCOPE_BACKFILL_OUT=/tmp/0028_body.sql \
--       cargo test -p fridge_backend scope_backfill -- --ignored --nocapture
--
-- The generator is `src/internships/scope_backfill.rs`, and it is the point of this file being
-- generated at all: the board slug is parsed by `dedup::ats_identity`, the same parser the rest
-- of the pipeline uses, which already knows that Greenhouse serves one board from
-- `boards.greenhouse.io`, `job-boards.greenhouse.io` and a regional host, and that this one
-- host's path may be case-folded. A second parser written in SQLite string functions would
-- agree with it until it did not, and the failure mode of that divergence is a sighting tagged
-- to a board that does not exist, waiting forever for a run that can never mention it.
--
-- WHY
--
-- 0026 tags a sighting when a run **sees** it. A sighting whose job is already gone can never
-- be seen, so it is never tagged; and an untagged sighting does not advance on a partial run,
-- which is nearly every Greenhouse run. Measured in 12j: of 42 legacy sightings on 100
-- completely enumerated boards, 37 were tagged and 5 were not, precisely because those 5 were
-- already dead. Scoped expiry is forward-looking, and this is what reaches backwards.
--
-- `posting_sightings.url` already records the board. `upsert_posting` rewrites that URL every
-- time the sighting is seen, so its slug is "the board that last reported this job" — the same
-- fact the tag written at fetch time carries, from the same run.
--
-- THE MEASUREMENT, 2026-09-03, over a copy of the live database
--
--   254 greenhouse sightings
--   253 taggable, across 82 boards, every one of them in the board directory
--     1 skipped: `ats_identity` returns the pseudo-slug `gh_jid` for a Greenhouse job known
--       only by a query-parameter id on a company's own careers page. `gh_jid` is not a board
--       and is never polled; tagging with it would make that row permanently unable to advance,
--       where untagged it can still advance on a fully successful run. The other pseudo-slug,
--       `embed`, is excluded for the same reason and did not occur here.
--     0 rows would be tagged with a slug the directory does not carry, so this backfill makes
--       no row worse off than leaving it untagged.
--
-- 2,087 sightings belong to unscoped sources (1,310 of them ATS-shaped). None are touched: a
-- scope on a source that reports none is a tag no run ever completes.
--
-- SAFETY
--
-- Every statement is `source = 'greenhouse' AND scope IS NULL`, so a tag written by a real run
-- always wins over a derived one, and a second application matches nothing. On a fresh database
-- none of these ids exist and the whole file is a no-op.
--
-- This narrows nothing. On a **fully successful** Greenhouse run an untagged sighting advances
-- because the source was completely enumerated; a tagged one advances because its board is in
-- the completed set — and on such a run every board is. The two are the same set, which
-- `a_tagged_and_an_untagged_sighting_advance_together_on_a_full_run` pins rather than assumes.

UPDATE posting_sightings SET scope = 'advancedspace'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('43bdefc6-0f0e-411e-8aef-be794a6af369', '25092e70-3f7d-449e-9939-1f5c575ba2f8', '7e90af23-39ae-4b6a-8f3c-c62d9b9dcd21', 'de8198ba-7933-4036-9555-7004048215a1');
UPDATE posting_sightings SET scope = 'andurilindustries'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('1cdeb55d-43a0-4bcb-a4e3-20ebeeb010be', '82549248-67b3-4d9c-92fe-279ab8755da7');
UPDATE posting_sightings SET scope = 'appian'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('c7ddabe1-96e4-40ba-b2d7-608d5eb7c2fe', '7efcdb42-7c0a-413c-8fa7-789db57f3726');
UPDATE posting_sightings SET scope = 'apptronik'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('29fcb10e-55b1-4def-b6b6-a5e779d907d2');
UPDATE posting_sightings SET scope = 'aquaticcapitalmanagement'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('aade32bb-c420-4012-bdc3-74dbc0439a98');
UPDATE posting_sightings SET scope = 'argmax'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('f65c2f16-5bfc-4876-8246-470c59860e20');
UPDATE posting_sightings SET scope = 'asteraearlycareer2026'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('543f1d66-454e-4f1c-87cd-6dc9dc89d5d0', 'ac1a7df3-0e39-420b-8335-733b138a46b2', 'd5cab017-2edd-46f7-8180-14b4cb8d19d9');
UPDATE posting_sightings SET scope = 'astranis'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('4646b21d-5d5c-42f2-886d-a17e6b7a465f', '55e25834-e27c-4453-8e9d-c25a8a114642', '2d6a4b58-69a6-456b-94d4-0d3ab1b7dc56', '799b0caa-1b61-4de9-af9d-9763e7095c19', '26d5ae87-388e-4151-b19a-1beeb04e8ce9', '49700cd8-107a-4899-be63-6cc20bed25f4', 'c850eeac-9e0c-488f-b4ef-13bff032ffb5', '832dedee-bd5e-4f1b-a293-3f108e2dfdd7');
UPDATE posting_sightings SET scope = 'audaxgroup'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('2ffad07c-8c75-4ca2-abab-c543fef4f71b', 'ba694535-05e3-46ea-8a38-dcf748372e03', 'f87620a8-dcfc-42a5-941d-a78cc0821521');
UPDATE posting_sightings SET scope = 'axontalentcommunity'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('63f0443b-b8ed-4e16-8905-c51a2053399d', 'f7e7eb98-141b-4d54-845b-aa0525491d52', '7b739dc3-1afe-46b7-9b71-d714d4b67702');
UPDATE posting_sightings SET scope = 'axq'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('ce056ef9-201d-4345-a420-e1e8affad4d5');
UPDATE posting_sightings SET scope = 'blackedgecapital'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('a35f6dc2-f227-431d-accf-d63ef0bcb4ab');
UPDATE posting_sightings SET scope = 'botauto'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('fbc0c06e-8267-4fa8-a730-d01001393eeb');
UPDATE posting_sightings SET scope = 'celonis'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('2c75c23e-5d8b-4637-9c1c-307dbc1615a8', 'c1574001-4218-41e3-8bff-c670d0247695', '7c558af7-8c41-41fb-8d2f-a4fb2ce3b591');
UPDATE posting_sightings SET scope = 'chicagotradingcampus'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('138edda7-071d-4f92-aae0-496f94689c0f');
UPDATE posting_sightings SET scope = 'clarityinnovates'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('84fa8442-a647-400b-990b-91a682bfa22a');
UPDATE posting_sightings SET scope = 'cloudflare'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('63ca68c9-66b1-46c5-82db-8f7e18312c08', 'fe0a5bf6-0f82-4240-b783-dfeb19c672a0', '2a01907a-4e39-4dfb-97c6-a8e53aa6771f', '7fe7f907-6bce-40d3-a261-6d5bd98577d1', '63051f7b-a057-44ef-959e-f36c29b98519', '5e1f2619-b46c-49fc-b5c4-17bf9ea3c8ac');
UPDATE posting_sightings SET scope = 'cresta'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('4cf034fb-3ee6-4101-b5ec-b3c7c4a215a8', 'e8a2a909-c4bf-4fea-bbfb-367de9f8aa4e', '96b3c061-e471-42cd-b782-5e7d85369090');
UPDATE posting_sightings SET scope = 'cssmerge'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('74f2aa41-f32c-44b8-bf51-e16febff250d', '83194419-101a-4166-9e61-69bc8badb0df', '97af6ada-9fdc-47a2-98b9-5c4510615b1c', 'f6968bd5-1556-4876-8427-33fb7f9dd66b');
UPDATE posting_sightings SET scope = 'ctccampusboard'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('cdb3252f-71e4-4df3-9c85-86e792050d60');
UPDATE posting_sightings SET scope = 'defenseunicorns'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('ceacb273-b29b-497e-a13a-1ecac22b5255');
UPDATE posting_sightings SET scope = 'didi'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('37d0b994-72cc-4703-8e19-41b424ad9486');
UPDATE posting_sightings SET scope = 'docugami'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('5a67d13c-3d7a-4340-ae62-762e8aced78a');
UPDATE posting_sightings SET scope = 'drweng'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('560daf0f-8656-42c9-89e9-e6a0e21a7d61', '0274c899-a336-4b26-89fe-855741e93b71', 'e7a30de8-295e-4aa7-bb0b-c92970d30311', 'a923ee8e-68b1-4669-b6e7-f116201968b1', '75d1ec65-6793-4161-a5ac-cdfd891d7340', '8f96a862-746f-4a34-8b1c-9aab6cdcdb3b', '3d18264a-d0f7-490f-beb0-662893b8cffe', '6ee78e04-147b-4324-a3d7-7f31fd066d7e');
UPDATE posting_sightings SET scope = 'drwuniversityjobs'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('ff0b2acf-4d83-40b3-92fa-f39d35602776');
UPDATE posting_sightings SET scope = 'dvtrading'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('8cf51c28-69a7-4bac-9b74-723ceef79472', '01fccb8b-eb5d-40a5-809d-f5c9ce0e73e9');
UPDATE posting_sightings SET scope = 'eulerity'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('5187b070-9a5a-41d5-9baa-dba7d83584ae', '7b0f0b32-5597-4ec4-8051-c746955d11f1', '3d46ae11-b16d-4b4f-9c1b-9755ad3987aa');
UPDATE posting_sightings SET scope = 'figma'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('e4f43e82-fb06-4901-8825-2de534dcaed7');
UPDATE posting_sightings SET scope = 'fiveringsllc'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('d1fbf04d-29bc-4129-969e-d9cfc1160727');
UPDATE posting_sightings SET scope = 'freeformfuturecorp'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('1fff75d4-a2bf-4575-bd9a-385046d1492c', '36a16c92-7ebf-41e3-9b49-df51aad80ec4', 'b4ba9349-e1b5-46d4-abb9-16f57345354f');
UPDATE posting_sightings SET scope = 'galaxydigitalservices'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('d662a8cb-a938-4ad6-bb18-38151232c61a');
UPDATE posting_sightings SET scope = 'gardacp'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('a3194d00-eba2-4bc3-8d9f-a97a32d9222e', '21f31f83-2da3-405e-b515-4a2d8664e6f4');
UPDATE posting_sightings SET scope = 'generalmatter'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('8aaf2f2a-c92f-4b03-8035-4b1742692e9e', 'd9845bfc-7843-4a7a-aa82-4e304ac2b210', 'a7a4fc46-1cfb-4587-b7ac-975854d897d4');
UPDATE posting_sightings SET scope = 'ginkgobioworks'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('624a5449-971e-4969-b1e4-08a83603c2a1', '23e6ea02-f1d2-4d1b-b49c-91beae4fb43a');
UPDATE posting_sightings SET scope = 'glossgenius'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('e06506b1-62eb-446b-8341-caa6b73a936c', '5e346136-83ba-4e91-b289-02d4369e320e');
UPDATE posting_sightings SET scope = 'haizelabs'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('141e52f6-1811-4281-8097-c87ccb6c694a');
UPDATE posting_sightings SET scope = 'honehealth'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('d874fd42-53c0-4c5c-bb0f-d2c4c9166051');
UPDATE posting_sightings SET scope = 'hyannisportresearch'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('4a5d3cf6-da58-4ec6-8a89-84ace6106873', 'a862eb24-e38b-4bfb-9e5c-0439e24d43a2');
UPDATE posting_sightings SET scope = 'incidentiq'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('dcd69bdd-c31f-411e-80e0-75f5d13c050e');
UPDATE posting_sightings SET scope = 'instalilyai'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('73bb1164-eb62-4e30-9e6c-79e2d3dfe8ff');
UPDATE posting_sightings SET scope = 'instead'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('043db6c5-daf6-4307-be43-3209cd8c4a07');
UPDATE posting_sightings SET scope = 'internshiplist2000'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('9be26b06-ddf0-43a5-9ee7-627fe9b0ca72', '84e0e9fc-8fea-4d98-9113-be44ffb39c99', '66b46e97-2d39-43c1-a7c5-f96050af8447', '7899465f-4bfe-4138-82dd-d3ebb8da9756', 'af935c6b-e505-4c5c-90f6-b2732b9f2a05', '0a5d9348-894d-4a0d-922a-67d52dedb4a9', 'da78780a-8931-4ab9-be0a-a07614b45ee9');
UPDATE posting_sightings SET scope = 'juvare'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('7e60b570-0203-4653-8fab-ac63b2a3d890');
UPDATE posting_sightings SET scope = 'kodiak'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('912b2206-3a8d-4a6a-a517-3cbd57d995b1');
UPDATE posting_sightings SET scope = 'later'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('36cde1a0-d315-40ba-98e4-64439210b323');
UPDATE posting_sightings SET scope = 'lilasciences'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('955e9476-e05e-4cce-8699-3e876b8d5d0e', '7203a3c9-b157-4467-8177-1bf2b8f5bfec', '62f55d9d-bc7a-480d-8a0e-9cc7cb438c74');
UPDATE posting_sightings SET scope = 'mavensecuritiesholdingltd'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('6b8dafac-d1f4-441a-aaa4-a173fa42e748');
UPDATE posting_sightings SET scope = 'metoxinternationalinc'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('9e021f70-fe0d-46e0-af43-56ddd67c0261');
UPDATE posting_sightings SET scope = 'momentenergy'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('60b5efa5-9e3c-4190-ae68-b03958c21a30');
UPDATE posting_sightings SET scope = 'morsecorpcoop'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('d7d9fc52-edf1-47d8-9f5a-e680c119f7d7', '35490b74-72e8-4a24-a594-a06a735a419d', '77f88c7e-8694-4105-9e8e-df1454a2828e', 'ccf9a75b-3bc2-40e6-a712-ed79c414a110', '4d47610a-0767-4ac9-8540-e5fe36e5a5ba', 'c49e6b4a-5175-46b5-a298-5e05ed78e1ad', '4f70f74e-cfa1-4c23-83ac-ea89986d09af', 'aca5a6c2-7dac-4c27-bfcf-fcb2f0ac67fc', 'a422b54b-be04-42b1-97c5-2a273aea0570', 'be02f339-d081-4b22-89eb-272a7625a56f', '16c99bd6-164d-433d-8699-2fd4c7d0150e', 'b3674d1e-7a19-45ff-b890-9bf9659aa921', 'fa324bbb-35c1-412e-bf32-dec1074240a6');
UPDATE posting_sightings SET scope = 'neuralink'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('7079c4af-f83e-402b-8300-fa5b7afc1745', 'befe880f-f558-40bc-b590-5a814e5477a4', '42eae788-17ee-4c2c-911e-cfbf1648e02d', '2fcaf287-d6de-4e3b-be30-6b38f1449c43', '5e9cc8ca-36b0-4a10-b9f7-2313e6bb1cf1', 'a09ef89f-e838-45c8-b499-f6b88ee1076d', 'b2bf3fda-328e-41d8-9444-599ef630e10e', '9c03da39-d639-40e8-aa23-1dd8a214093b', 'cc34df6b-869b-4599-a23c-263762ac6ef8', 'fb6b3219-d4c1-48e6-9d07-c1232d6162c5');
UPDATE posting_sightings SET scope = 'newsbreak'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('574bcd47-2f87-42ab-9b52-34a59e0fe065');
UPDATE posting_sightings SET scope = 'nisc'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('01a0984a-75c1-436e-b31f-14c7b4ca2795', '33254c04-b3ae-4022-8cc5-50f9526401ac');
UPDATE posting_sightings SET scope = 'olsson'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('bad5ed3f-0213-4efb-86b7-cb09732b4839', '646f0dfa-5102-436c-9d7c-bd3fc21f7199', 'aee40086-c5cc-406d-9740-3f80aa4397ef', '4f141527-69ac-4fc6-8b28-299c31289971', '445f0f91-0446-4b34-853c-fbfa31c60545', '50ccd58d-f732-4c1d-a359-95a5d9945f73', '51179a30-f17e-427f-9828-207ef622c1e4', 'db22c499-07cc-42ef-97f7-f883a2d7f79e', '9b1b48ec-4d60-4aff-8ae0-e2a3684fa122', '3945cd57-29e2-4c1b-bbcb-bbf23463b4d0', 'e4d87de1-50b7-4caf-aaaf-29da5ad7d416', '364f1ae1-7be2-4690-a685-7152e26d1661', '509c1a68-5af1-4cb8-ae00-249aa6b50272', 'ea3b4e9c-fda1-4952-8600-dc1e4ef23c98', 'f8bb3a2d-4efb-48ea-be6d-eb8f2b838dc4');
UPDATE posting_sightings SET scope = 'omnicomhealth'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('2d149b80-4842-463d-80b2-11e85ff48d89');
UPDATE posting_sightings SET scope = 'pdtpartners'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('8d32d26c-ac72-4a6a-aba2-aac9f9b33968', 'f32c4bdd-3094-4a12-beab-e9ad0c295b87');
UPDATE posting_sightings SET scope = 'point72'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('1dac6418-a331-4931-8e72-4977f113714d', 'bfe772c8-1837-4f6b-a172-b8b026bd7958', 'a95b713e-d146-482e-818b-c6432a7dc104');
UPDATE posting_sightings SET scope = 'postman'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('eff005d9-ffd2-4cf6-8bde-364448bfff3a');
UPDATE posting_sightings SET scope = 'purestorage'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('95c0ce78-fb91-40b2-a506-9521ca4a7eda');
UPDATE posting_sightings SET scope = 'rackner'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('f7760618-e170-4a65-8321-b571da833c8c');
UPDATE posting_sightings SET scope = 'rocketlab'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('ef7d6490-2bf5-476d-b47f-3ad68c88c785', 'ffd14466-4f93-4806-aa47-4c16ec9ce743', '0b81ed38-44e5-418b-a703-021b4701df1c', 'aa9bbe94-1f38-4ec4-aef1-280a3a1cb181', '77a230ee-88aa-40c5-828a-6efbf009d6a1', '6edbcc70-a96c-4119-b1dc-148dd4bd4aac', '33eb32f3-5ade-4bd4-968f-07fd3034a8ab', '496056db-6c28-4c8b-b799-d4f36bc62af6', 'a07ddb4a-32f8-426a-afac-4caefc02733a', '6ae68bb6-c0ca-48a9-b1ed-999268a721e3', '2e9fe218-ef34-4f02-990e-45130a6acdd8', 'db40f9ff-b376-486a-a926-d97807802b3d', '0951c1bc-b635-4e7c-a6f3-7425aa878226', '50b27269-28b1-420d-ac63-ca3328930afb');
UPDATE posting_sightings SET scope = 'schonfeld'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('76b3ab37-3671-457a-860d-98e7fae52e62');
UPDATE posting_sightings SET scope = 'sevenresearch'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('0bd19e42-5376-4a52-bbe2-d051f9108786', 'e87ff56d-c9b4-4f8b-a4a7-55754e630d69');
UPDATE posting_sightings SET scope = 'sezzle'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('ecf042b8-2480-42ff-a7f0-75e66c573d1c', 'd0a29ed2-4cf5-4822-b5ce-1ea7c71dd8db', 'c777174a-ba9a-439e-b4f1-204389d13655', '29aaefaf-f606-4d23-b1f6-6b055e158db3', 'ebe96cee-aa78-40cc-aae4-cd952557c378', 'ce4939da-9d28-4c8d-abd5-5875754470ee', 'dbc150b8-559b-4cb3-8dbc-c9d239dee530', '822b80e6-331f-48fd-8605-c557218e4b86', '5f4a6951-d67c-45b9-b9f8-f5e38757de1d', '5e45b71f-e5fd-475b-8cb9-7192a8dcc127', '2fc8dbe2-6319-4091-af91-ae9faf4211ce', '8f6569c7-11ac-4fa5-a644-ecdbe02248a9', 'd27645d2-828d-4620-928a-19317972af6b', 'af075e9c-36c9-42d5-86c8-1a277cb6ffba');
UPDATE posting_sightings SET scope = 'sharkninjaoperatingllc'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('8ca03a8c-e6c5-4e11-bd74-d94e9bf6dbff', '56458021-f08d-4dea-bec5-2ea9f77d1d83');
UPDATE posting_sightings SET scope = 'solidpower'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('a2405db2-c1ca-4ec3-b126-d044d7d01bfb', 'cbd09537-8b9f-4303-9c71-4c91a71e2042');
UPDATE posting_sightings SET scope = 'spacex'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('c5e10cfe-480f-4f97-befb-122326f1ad67', 'e50c3fd0-81d6-4d56-a726-8e1f56384774', 'b821bf6a-e704-4d19-ba86-d155c5ebe62e', '4b4c3cb2-e445-4294-a309-ac2183643414', '1869ee39-933d-450a-8664-29b380b9d8f2', 'fe6d7fd6-0639-445b-9d26-428850b65c4b', 'c0c8fb7c-3d88-4a54-bd51-fadc75ecd12d', '04d064b6-9017-4d89-9a33-99e320fddc70');
UPDATE posting_sightings SET scope = 'tenstorrentuniversity'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('b3405b78-ba42-4e06-8820-0a6953d48d89', 'efc23429-cebf-43d2-91d2-0f0fa6613b7c', 'c3451789-8f09-4515-b22a-c06da85a2f18', '5a0400a0-e4cb-4225-a821-50281c495360', 'b91f6beb-bedb-40b2-a016-801e26c99fd8', 'b78c424a-017d-4557-8ddf-2a5a580a7c97', '8d21b9cb-5b7a-412d-9de8-dd8a83170e1c', '379db0c4-1850-4da7-abf8-1f8d51cde630', '48a360c0-02f4-4471-87f0-22a7c0e10fa2', 'edf03b11-4aad-4aa1-8752-03bc31e49d06', '3b758c39-263a-475a-92fa-7a692bbc058c', '56acfe5e-10e9-4d68-8254-707174a70e83', '9f7a4287-fc1c-4bec-8c61-4eea8ceac019', '4793a3e3-f91c-410d-9f93-54df639cd60d');
UPDATE posting_sightings SET scope = 'testnisc'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('5f53b4b8-d709-4246-a3b3-f1bd207d3227', 'd6bb577f-37a4-4a0f-bb8b-daed5ba07d57', '5e6df017-b4e8-4197-a36a-8ab9595c147d', '5006d978-b66e-432f-9e0c-33bf775042f7', '492ed7ba-2357-4528-aa9d-2b4f397f5692', 'c947fe4b-1c1f-45e4-9974-6929fbac5577');
UPDATE posting_sightings SET scope = 'thenuclearcompany'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('2a0ba54f-6858-42b1-b6e3-311885e354d9', 'b6b1e734-60b5-4183-9826-717c87cf3976', '43352a17-4b36-4e93-beb9-bf528a04f893', 'f802e6ac-d115-4fc3-98ff-c7eea0d74cf5', '1e293ea4-fd85-44e1-90ac-91cca03c5b47');
UPDATE posting_sightings SET scope = 'thetradedesk'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('9d6c1436-b6d7-4e58-83bb-ab156fae1e85');
UPDATE posting_sightings SET scope = 'transmarketgroup'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('8109bb22-7d6c-41a9-a7e0-23d70f59e1ab', '0ea4ae1e-a373-4cef-923b-2400b36c1472');
UPDATE posting_sightings SET scope = 'tribalscale'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('7be37c70-c69a-4581-9329-48f6377ed70b');
UPDATE posting_sightings SET scope = 'trueanomalyinc'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('e8c11a63-23b1-4fac-a14c-c746adecaa40');
UPDATE posting_sightings SET scope = 'truveta'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('45b1490e-10aa-4efa-9fbb-b0560744e42e');
UPDATE posting_sightings SET scope = 'vardaspace'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('9b3370b5-4eea-41b7-9e82-ce957efff24e', '0f4ab7c9-e869-4fc9-baeb-1bcab8c02118', 'd33261ba-46b3-44b1-be89-c432ee3823f0', '3bdcb5d3-3bc3-404a-b37a-e2f0e2c29d89', '781de0b1-25f8-42ca-9e34-123dcee22ccb', '4cbdb6ab-69d3-4811-888e-b1d63152db0b', 'd01b205c-fcbc-4672-ba94-be38ab8201b8');
UPDATE posting_sightings SET scope = 'verkada'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('98e5cad5-d1fb-480b-82ed-34327cd8b31a', '29ddf3aa-ffb7-421f-b1d8-50062cefe40c', 'b0ffc65d-4c6f-42a7-b79e-1fbf6e445a32', '92a29961-e26e-4ac7-853b-ed10b1b52fbd', 'c22604cc-0e48-4150-9b50-42fa5eb288c1');
UPDATE posting_sightings SET scope = 'virtu'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('9217cc2c-2dba-490a-9ddd-f77d29690be5', 'a4f62b0c-3430-46ba-ad45-6227eb354730', '5e94c13c-7279-4bf0-b3b5-e33d7b81d868', '95b0e7ea-b201-4e5a-8e68-37c2c5fa2c7c', 'e19f0a45-c2c3-4afd-8f72-3725f33a93bd', '1dcd470f-ac6c-4b75-9da8-2cc18c48718b');
UPDATE posting_sightings SET scope = 'voloridgeinvestmentmanagement'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('36028d03-6339-4e7a-87a0-70ef88711d4c');
UPDATE posting_sightings SET scope = 'walleyecapital-external-students'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('47cf067b-802a-44be-a0d1-60f113752ac1');
UPDATE posting_sightings SET scope = 'xantium'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('ecbc57f7-90e9-42da-a90f-67188d82d643');
UPDATE posting_sightings SET scope = 'zscaler'
 WHERE source = 'greenhouse' AND scope IS NULL AND id IN ('8c168ee0-ac68-402e-9534-e266041ad2f0', 'da12d14b-cf99-4628-b22a-3900132cf094', '6739b00a-930b-42fe-9a03-d6aa10ac5298');
