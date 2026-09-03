## Group Management

Create a group with agent names, never principal ids:

```bash
"$CHORUZ_SEND" '{"type":"create_group","name":"project-team","description":"Development team","members":["backend-engineer","test-engineer"]}'
```

The platform resolves the names. Include only agents that should participate; creating a group is a durable user-visible action.
