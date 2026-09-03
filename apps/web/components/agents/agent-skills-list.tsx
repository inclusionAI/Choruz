"use client";

import { X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { trace } from "../../lib/api/choruz-trace";
import type { RuntimeBindingInfo } from "../../lib/api/choruz-types";
import { apiBaseUrl } from "../../lib/api/choruz-api";
import { Spinner } from "../ui/spinner";
import { EmptyState } from "../ui/empty-state";
import { transportFetch } from "../../lib/api/transport";

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export type SkillInfo = { name: string; path: string; type?: "command" | "skill"; description?: string | null };

// ---------------------------------------------------------------------------
// AddSkillPanel
// ---------------------------------------------------------------------------

function AddSkillPanel({
  binding,
  onInstalled,
  onClose,
}: {
  binding: RuntimeBindingInfo;
  onInstalled: () => void;
  onClose: () => void;
}) {
  const [localPath, setLocalPath] = useState("");
  const [localBusy, setLocalBusy] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);

  const handleLocalImport = async () => {
    if (!localPath.trim()) return;
    setLocalBusy(true);
    setLocalError(null);
    try {
      const res = await transportFetch(`${apiBaseUrl()}/agent-skills`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          workspace_path: binding.workspace_path,
          source: "local",
          local_path: localPath.trim(),
        }),
      });
      if (!res.ok) {
        const data = await res.json().catch(() => ({})) as { error?: string };
        throw new Error(data.error || "Import failed");
      }
      setLocalPath("");
      onInstalled();
    } catch (err) {
      setLocalError(err instanceof Error ? err.message : "Import failed");
    } finally {
      setLocalBusy(false);
    }
  };

  return (
    <div className="add-skill-panel">
      <div className="add-skill-header">
        <h4>Add Skill</h4>
        <button type="button" className="delete-btn" onClick={onClose} title="Close" aria-label="Close add skill"><X size={14} aria-hidden="true" /></button>
      </div>

      <div className="add-skill-local">
        <input
          type="text"
          placeholder="/path/to/skill.md"
          value={localPath}
          onChange={(e) => setLocalPath(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && handleLocalImport()}
          className="add-skill-input"
        />
        <button
          className="agent-config-save-btn"
          onClick={handleLocalImport}
          disabled={localBusy || !localPath.trim()}
          aria-busy={localBusy}
          aria-label="Import skill"
        >
          {localBusy ? <Spinner size={12} /> : "Import"}
        </button>
        {localError && <p className="add-skill-error">{localError}</p>}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// AgentSkillsList
// ---------------------------------------------------------------------------

export function AgentSkillsList({
  binding,
}: {
  binding: RuntimeBindingInfo;
}) {
  const [skills, setSkills] = useState<SkillInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [showAdd, setShowAdd] = useState(false);
  const [expandedSkill, setExpandedSkill] = useState<string | null>(null);
  const [skillContent, setSkillContent] = useState<string | null>(null);
  const [loadingContent, setLoadingContent] = useState(false);

  const loadSkills = useCallback(async () => {
    setLoading(true);
    try {
      const params = new URLSearchParams({
        workspace_path: binding.workspace_path,
      });
      const res = await transportFetch(`${apiBaseUrl()}/agent-skills?${params}`);
      if (!res.ok) throw new Error("Failed to load skills");
      const data = await res.json() as { skills: SkillInfo[] };
      setSkills(data.skills);
    } catch {
      setSkills([]);
    } finally {
      setLoading(false);
    }
  }, [binding.workspace_path]);

  useEffect(() => {
    loadSkills();
  }, [loadSkills]);

  const handleToggleExpand = async (skillName: string) => {
    if (expandedSkill === skillName) {
      setExpandedSkill(null);
      setSkillContent(null);
      return;
    }
    setExpandedSkill(skillName);
    setSkillContent(null);
    setLoadingContent(true);
    try {
      const params = new URLSearchParams({
        workspace_path: binding.workspace_path,
        read: skillName,
      });
      const res = await transportFetch(`${apiBaseUrl()}/agent-skills?${params}`);
      if (!res.ok) throw new Error("Failed to read skill");
      const data = await res.json() as { content: string };
      setSkillContent(data.content);
    } catch {
      setSkillContent("Failed to load content.");
    } finally {
      setLoadingContent(false);
    }
  };

  const handleDelete = async (skillName: string) => {
    if (!confirm(`Delete skill "${skillName}"?`)) return;
    setDeleting(skillName);
    try {
      const res = await transportFetch(`${apiBaseUrl()}/agent-skills`, {
        method: "DELETE",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          workspace_path: binding.workspace_path,
          skill_name: skillName,
        }),
      });
      if (!res.ok) throw new Error("Failed to delete");
      setSkills((prev) => prev.filter((s) => s.name !== skillName));
      if (expandedSkill === skillName) {
        setExpandedSkill(null);
        setSkillContent(null);
      }
    } catch (err) {
      trace.event("skill_delete_error", { skillName, error: String(err) });
    } finally {
      setDeleting(null);
    }
  };

  if (loading) {
    return (
      <div className="detail-section">
        <h4>Skills</h4>
        <p className="detail-inline-empty"><Spinner label="Loading…" /></p>
      </div>
    );
  }

  return (
    <div className="detail-section">
      <div className="agent-config-header">
        <h4>Skills ({skills.length})</h4>
        <button
          className="agent-config-save-btn"
          onClick={() => setShowAdd(!showAdd)}
          title="Add skill"
        >
          {showAdd ? "Cancel" : "+ Add"}
        </button>
      </div>

      {showAdd && (
        <AddSkillPanel
          binding={binding}
          onInstalled={() => {
            loadSkills();
          }}
          onClose={() => setShowAdd(false)}
        />
      )}

      {skills.length === 0 && !showAdd ? (
        <EmptyState inline description="No skills installed." />
      ) : (
        skills.map((s) => (
          <div key={s.name}>
            <div className="skill-row">
              <span
                className="skill-icon"
                style={{ cursor: "pointer" }}
                onClick={() => handleToggleExpand(s.name)}
              >
                {expandedSkill === s.name ? "\u25BC" : "\u25B6"}
              </span>
              <div
                style={{ flex: 1, cursor: "pointer", minWidth: 0 }}
                onClick={() => handleToggleExpand(s.name)}
              >
                <span className="skill-name">{s.name.replace(/\.md$/, "")}</span>
                {s.description && (
                  <span
                    style={{
                      display: "block",
                      fontSize: "11px",
                      color: "var(--text-muted)",
                      overflow: "hidden",
                      textOverflow: "ellipsis",
                      whiteSpace: "nowrap",
                    }}
                  >
                    {s.description}
                  </span>
                )}
              </div>
              <button
                className="delete-btn"
                title={`Delete ${s.name}`}
                disabled={deleting === s.name}
                aria-busy={deleting === s.name}
                onClick={() => handleDelete(s.name)}
              >
                {deleting === s.name ? <Spinner size={12} /> : <X size={14} aria-hidden="true" />}
              </button>
            </div>
            {expandedSkill === s.name && (
              <div className="skill-content-panel">
                {loadingContent ? (
                  <p className="detail-inline-empty"><Spinner label="Loading…" /></p>
                ) : (
                  <pre className="skill-content-pre">{skillContent}</pre>
                )}
              </div>
            )}
          </div>
        ))
      )}
    </div>
  );
}
