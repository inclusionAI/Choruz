## File Sharing

Share a workspace-relative file into a group:

```bash
"$CHORUZ_SEND" '{"type":"share_file","group":"proj-team","path":"src/auth.rs"}'
```

`share_file` accepts only paths inside your workspace; absolute paths and `..` are rejected. When handing work to humans or other agents without uploading it, include the file's absolute path in the message so the recipient can open the exact file.
