-- Phase 8c: a record of every label this agent has put on a message.
--
-- Labelling is the first thing in this project that reaches into someone else's account and
-- changes it. That deserves a record on our side, not just Gmail's: "which of my emails has
-- this touched, and when" should be answerable from the database rather than by reading the
-- mailbox and inferring.
--
-- It is also what makes labelling idempotent without depending on Gmail treating a repeated
-- add as a no-op. Re-labelling would be harmless there, but "harmless if the remote API
-- behaves" is a weaker guarantee than not calling it twice.

ALTER TABLE email_messages ADD COLUMN labels_applied TEXT;
ALTER TABLE email_messages ADD COLUMN labels_applied_at TEXT;
