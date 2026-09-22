## Reviewed Browser Workflows

Before repeating a browser task, discover this Agent's human-approved workflows:

```bash
"$CHORUZ_SEND" '{"type":"browser_workflows"}'
```

Commands are asynchronous. In headless sessions they are processed only after your turn returns: issue the command and return control, never wait inside that turn for its result. Inspect the durable non-chat result in `.choruz-outbox/results/` on a later turn; these commands do not automatically start another Agent turn. Use a returned grant only when its `ready` state and applicability match the current request. A match is not permission to change the workflow or perform unrelated work. If none applies, use the normal authorized browser tools.

```bash
"$CHORUZ_SEND" '{"type":"browser_workflow_run","binding_id":"RETURNED_BINDING","grant_id":"RETURNED_GRANT","run_id":"UNIQUE-STABLE-RUN-ID","task":"The current requested browser task","values":{"approved_field":"current input"}}'
"$CHORUZ_SEND" '{"type":"browser_workflow_status","binding_id":"RETURNED_BINDING","grant_id":"RETURNED_GRANT","run_id":"SAME-RUN-ID"}'
```

Supply exactly the manifest's `values` keys; do not supply generated fields. The platform matches the task, enforces authorization, runs the approved steps and checks the result. A queued or `running` receipt is not completion. Request `browser_workflow_status` on a later turn, not another run. Sending run and status together only reads the current status; it does not wait for completion. Keep the same run id after a transport interruption. A failed or uncertain outcome requires inspection; do not bypass it with a new grant or retry through other browser tools.

Report the observed result and preserve failure evidence in the task trace. Propose repairs through the existing learning process; changed workflows require a new live validation and human authorization. Never claim that a saved rule, successful match or acknowledged click proves the whole task succeeded.
