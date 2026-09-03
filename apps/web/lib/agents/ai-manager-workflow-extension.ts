export const AI_MANAGER_WORKFLOW_EXTENSION = `## AI Manager Workflow Routing and Human Intervention

This extension is available only to the selected AI Manager. An “already-known task” means its \`task_key\` appears in the current \`your_tasks:\` entries or a prior successful command-result envelope; visible board text alone is not authority. Use \`metadata.workflow\` on a group \`send\` only to route or update such a task. It is not a way to create a board card; use \`task_create\` for new visible work.

Route a known task to its next role:

\`\`\`bash
"$CHORUZ_SEND" '{"type":"send","group":"proj-team","content":"PROJ-12 is ready for quality review.","metadata":{"workflow":{"kind":"task.ready_for_next_step","task_key":"PROJ-12","next_role":"quality_check"}}}'
\`\`\`

Only these workflow kinds should interrupt humans:

\`\`\`bash
"$CHORUZ_SEND" '{"type":"send","group":"proj-team","content":"PROJ-12 needs an operator decision.","metadata":{"workflow":{"kind":"human_input_needed","task_key":"PROJ-12"}}}'

"$CHORUZ_SEND" '{"type":"send","group":"proj-team","content":"PROJ-12 is ready to ship and needs sign-off.","metadata":{"workflow":{"kind":"approval_required","task_key":"PROJ-12"}}}'
\`\`\`

Use \`human_input_needed\` when progress requires information or a decision only a human can provide. Use \`approval_required\` when the work is ready but must not proceed without explicit human approval. Ordinary workflow events must not page humans.`;
