-- Applications created from mail were dated when the button was pressed, not when they happened.
--
-- Phase 12s, repairing 0033's own writes. `untracked::decide` stamped `applied_at = now()`, so
-- eleven applications made between 30 August and 4 September 2026 all landed in a single
-- 5 September cohort.
--
-- That is not cosmetic. `applied_at` is the cohort key for the analytics monthly breakdown, the
-- left edge of every time-to-first-response measurement, and the column the analytics date
-- window filters on. Dating them late compresses eleven real applications into one day and
-- shortens every response time by however long the row sat unaccepted — and for an application
-- created at `oa`, it can place the response before the application.
--
-- A confirmation email lands essentially at apply time, so the message's `received_at` is the
-- best available estimate, and it is already stored. `snapshot_json.verdict_id` is the link,
-- written by the same code that created the row.
--
-- Rows whose evidence no longer resolves are left exactly as they are: `now()` was a guess, and
-- replacing it with nothing would be worse than a dated guess. The WHERE clause is what makes
-- this migration safe to run against a database where the chain is broken.

UPDATE internship_applications
   SET applied_at = (
           SELECT m.received_at
             FROM email_verdicts v
             JOIN email_messages m ON m.id = v.message_id
            WHERE v.id = json_extract(internship_applications.snapshot_json, '$.verdict_id')
       ),
       status_changed_at = (
           SELECT m.received_at
             FROM email_verdicts v
             JOIN email_messages m ON m.id = v.message_id
            WHERE v.id = json_extract(internship_applications.snapshot_json, '$.verdict_id')
       )
 WHERE source = 'email'
   AND json_extract(snapshot_json, '$.verdict_id') IS NOT NULL
   AND (
           SELECT m.received_at
             FROM email_verdicts v
             JOIN email_messages m ON m.id = v.message_id
            WHERE v.id = json_extract(internship_applications.snapshot_json, '$.verdict_id')
       ) IS NOT NULL;

-- The creation events carry the same wrong instant, and `metrics_for` folds events by time. If
-- the event stayed at the later timestamp while the application moved earlier, an application
-- created at `oa` would report a response days after an application that had not happened yet.
UPDATE application_events
   SET at = (
           SELECT a.applied_at FROM internship_applications a WHERE a.id = application_events.application_id
       )
 WHERE actor = 'email'
   AND from_status IS NULL
   AND EXISTS (
           SELECT 1 FROM internship_applications a
            WHERE a.id = application_events.application_id AND a.source = 'email'
       );
