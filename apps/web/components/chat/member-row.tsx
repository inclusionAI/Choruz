"use client";

import { useCallback, useState } from "react";
import { ChevronDown, X } from "lucide-react";
import type { RuntimeBindingInfo } from "../../lib/api/choruz-types";
import type { SkillInfo } from "../agents/agent-skills-list";
import { Avatar } from "../ui/avatar";
import { apiBaseUrl } from "../../lib/api/choruz-api";
import { driverDisplayName } from "../../lib/drivers/driver-registry";
import { HarnessAccountSummary } from "../agents/harness-account-summary";
import { transportFetch } from "../../lib/api/transport";

// ---------------------------------------------------------------------------
// MemberRow
// ---------------------------------------------------------------------------

export type MemberInfo = {
  id: string;
  name: string;
  isAgent: boolean;
};

export function MemberRow({
  member,
  isSelf,
  binding,
  isGroup,
  machine,
  canRemove,
  onRemove,
  showSkills = true,
}: {
  member: MemberInfo;
  isSelf: boolean;
  binding?: RuntimeBindingInfo;
  isGroup: boolean;
  /** Where the agent runs, from `bindingMachineLabel`; shown as a badge. */
  machine?: string;
  canRemove?: boolean;
  onRemove?: (member: MemberInfo) => Promise<void> | void;
  showSkills?: boolean;
}) {
  const [expanded, setExpanded] = useState(false);
  const [skills, setSkills] = useState<SkillInfo[] | null>(null);
  const [loadingSkills, setLoadingSkills] = useState(false);
  const [removing, setRemoving] = useState(false);

  const loadSkills = useCallback(async () => {
    if (!showSkills || !binding?.workspace_path || skills !== null) return;
    setLoadingSkills(true);
    try {
      const params = new URLSearchParams({ workspace_path: binding.workspace_path });
      const res = await transportFetch(`${apiBaseUrl()}/agent-skills?${params}`);
      if (res.ok) {
        const data = await res.json() as { skills: SkillInfo[] };
        setSkills(data.skills);
      } else {
        setSkills([]);
      }
    } catch {
      setSkills([]);
    } finally {
      setLoadingSkills(false);
    }
  }, [binding?.workspace_path, showSkills, skills]);

  const handleToggle = useCallback(() => {
    if (!expanded && binding && member.isAgent && showSkills) {
      loadSkills();
    }
    setExpanded(!expanded);
  }, [expanded, binding, member.isAgent, loadSkills, showSkills]);

  const canExpand = isGroup && member.isAgent && binding;
  const showRemove = Boolean(canRemove && onRemove && !isSelf);

  const handleRemoveClick = useCallback(async () => {
    if (!onRemove || removing) return;
    if (!window.confirm(`Remove ${member.name} from group?`)) return;
    setRemoving(true);
    try {
      await onRemove(member);
    } finally {
      setRemoving(false);
    }
  }, [onRemove, member, removing]);

  const identity = (
    <>
      <Avatar name={member.name} size="small" />
      <span className="member-name">
        {member.name}
        {isSelf ? " (you)" : ""}
      </span>
      {member.isAgent && <span className="agent-badge">AI</span>}
      {member.isAgent && machine ? (
        <span className="member-machine" title={`Runs on ${machine}`}>{machine}</span>
      ) : null}
    </>
  );

  return (
    <div>
      <div className="member-row">
        {/* The identity block is the disclosure control when there is
            binding detail to show; the remove button stays outside it. */}
        {canExpand ? (
          <button
            type="button"
            className="member-row-main"
            aria-expanded={expanded}
            onClick={handleToggle}
          >
            {identity}
            <ChevronDown
              size={14}
              className={`disclosure-caret${expanded ? " is-open" : ""}`}
              aria-hidden="true"
            />
          </button>
        ) : (
          <div className="member-row-main">{identity}</div>
        )}
        {showRemove && (
          <button
            type="button"
            className="member-remove-btn"
            aria-label={`Remove ${member.name} from group`}
            title={`Remove ${member.name} from group`}
            disabled={removing}
            aria-busy={removing}
            onClick={handleRemoveClick}
          >
            <X size={14} aria-hidden="true" />
          </button>
        )}
      </div>

      {/* Expanded agent info */}
      {expanded && binding && (
        <div className="member-row-detail">
          <div className="member-row-field">
            <span className="member-row-label">Workspace</span>
            <div className="member-row-path">{binding.workspace_path}</div>
          </div>

          {machine ? (
            <div className="member-row-field">
              <span className="member-row-label">Machine</span>{" "}
              <span className="member-row-muted">{machine}</span>
            </div>
          ) : null}

          <div className="member-row-field">
            <span className="member-row-label">Driver</span>{" "}
            <span className="member-row-muted">{driverDisplayName(binding.driver_type)}</span>
            {" · "}
            <span className={`member-row-state${binding.state === "running" ? " is-running" : ""}`}>
              {binding.state}
            </span>
          </div>

          {binding.harness_account_id && binding.workspace_id ? (
            <HarnessAccountSummary accountId={binding.harness_account_id} companyId={binding.workspace_id} />
          ) : null}

          {showSkills && (
            <div className="member-row-field">
              <span className="member-row-label">Skills</span>
              {loadingSkills && <span className="member-row-muted"> loading…</span>}
              {skills !== null && skills.length === 0 && (
                <span className="member-row-muted"> none</span>
              )}
              {skills !== null && skills.length > 0 && (
                <div className="member-row-skills">
                  {skills.map((s) => (
                    <span key={s.name} className="member-row-skill">
                      {s.name}
                    </span>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
