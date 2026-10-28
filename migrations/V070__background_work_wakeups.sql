-- Wake background workers when their work becomes due instead of waiting
-- for the next poll. The payload names the worker; it carries no state, and
-- workers keep polling as the fallback when no listener is connected.
CREATE FUNCTION notify_background_work()
RETURNS TRIGGER AS $$
BEGIN
    PERFORM pg_notify('choruz_background_work', TG_ARGV[0]);
    RETURN NULL;
END;
$$ LANGUAGE plpgsql;

-- Saving learning settings or starting a new generation makes a check due.
CREATE TRIGGER trg_learning_check_due
    AFTER INSERT OR UPDATE OF next_check_at, enabled, lease_token ON experience_policy
    FOR EACH ROW
    WHEN (NEW.enabled AND NEW.lease_token IS NULL AND NEW.next_check_at <= NOW())
    EXECUTE FUNCTION notify_background_work('learning');

-- A queued case is claimable; a reviewed candidate may need its automatic
-- evaluation queued.
CREATE TRIGGER trg_evaluation_case_queued
    AFTER INSERT OR UPDATE OF status ON experience_evaluation
    FOR EACH ROW
    WHEN (NEW.status = 'queued')
    EXECUTE FUNCTION notify_background_work('evaluation');

CREATE TRIGGER trg_evaluation_candidate_saved
    AFTER INSERT ON experience_revision
    FOR EACH ROW
    WHEN (NEW.disposition = 'candidate')
    EXECUTE FUNCTION notify_background_work('evaluation');

-- New behavior evidence waits for preparation.
CREATE TRIGGER trg_behavior_event_saved
    AFTER INSERT ON experience_behavior_event
    FOR EACH ROW
    EXECUTE FUNCTION notify_background_work('behavior');
