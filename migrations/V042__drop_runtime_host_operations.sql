-- V042: a paired device answers filesystem and session-catalog requests over
-- its host link (`/v1/ws/runtime-hosts/link`); the leased request/response
-- queue has no readers left.
DROP TABLE IF EXISTS runtime_host_operation;
