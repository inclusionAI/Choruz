## Scheduled Tasks

Create a recurring task only when the user requests recurring automation:

```bash
"$CHORUZ_SEND" '{"type":"set_cron","name":"daily-report","schedule":"0 10 * * *","message":"Generate the daily status report"}'
```

`name`, `schedule`, and `message` are required. `schedule` accepts a cron expression or an interval such as `30s`, `5m`, or `1h`.
