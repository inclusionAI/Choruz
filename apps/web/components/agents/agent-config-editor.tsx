"use client";

import { useCallback, useEffect, useState } from "react";
import { trace } from "../../lib/api/choruz-trace";
import type { RuntimeBindingInfo } from "../../lib/api/choruz-types";
import { AgentInstructionForm } from "./agent-instruction-form";
import {
  emptyFields,
  fieldsToMarkdown,
  markdownToFields,
  type AgentInstructionFields,
} from "../../lib/agents/agent-instructions";
import { apiBaseUrl } from "../../lib/api/choruz-api";
import { Spinner } from "../ui/spinner";
import { transportFetch } from "../../lib/api/transport";

export function AgentConfigEditor({
  binding,
  sessionToken,
}: {
  binding: RuntimeBindingInfo;
  sessionToken: string;
}) {
  const [fields, setFields] = useState<AgentInstructionFields>(emptyFields());
  const [savedFieldsJson, setSavedFieldsJson] = useState("");
  const [filename, setFilename] = useState("");
  const [format, setFormat] = useState<"choruz" | "raw">("choruz");
  const [rawContent, setRawContent] = useState("");
  const [savedRawContent, setSavedRawContent] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saveMsg, setSaveMsg] = useState<string | null>(null);

  const loadConfig = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const params = new URLSearchParams({
        workspace_path: binding.workspace_path,
        driver_type: binding.driver_type,
      });
      const res = await transportFetch(`${apiBaseUrl()}/agent-config?${params}`);
      if (!res.ok) throw new Error("Failed to load config");
      const data = (await res.json()) as {
        filename: string;
        content: string;
        format?: "choruz" | "raw";
      };
      setFilename(data.filename);
      setFormat(data.format === "raw" ? "raw" : "choruz");
      setRawContent(data.content);
      setSavedRawContent(data.content);
      const parsed = markdownToFields(data.content);
      setFields(parsed);
      setSavedFieldsJson(JSON.stringify(parsed));
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to load");
    } finally {
      setLoading(false);
    }
  }, [binding.workspace_path, binding.driver_type]);

  useEffect(() => {
    loadConfig();
  }, [loadConfig]);

  const handleSave = async () => {
    setSaving(true);
    setError(null);
    setSaveMsg(null);
    const span = trace.start("config_save", { workspacePath: binding.workspace_path, driverType: binding.driver_type });
    try {
      const content = format === "raw" ? rawContent : fieldsToMarkdown(fields);
      const res = await transportFetch(`${apiBaseUrl()}/agent-config`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({
          workspace_path: binding.workspace_path,
          driver_type: binding.driver_type,
          content,
        }),
      });
      if (!res.ok) throw new Error("Failed to save");
      span.end({ status: res.status });
      setSavedFieldsJson(JSON.stringify(fields));
      setSavedRawContent(rawContent);
      setSaveMsg("Saved");
      setTimeout(() => setSaveMsg(null), 2000);
    } catch (err) {
      span.end({ error: err instanceof Error ? err.message : String(err) });
      setError(err instanceof Error ? err.message : "Failed to save");
    } finally {
      setSaving(false);
    }
  };

  const isDirty = format === "raw"
    ? rawContent !== savedRawContent
    : JSON.stringify(fields) !== savedFieldsJson;

  if (loading) {
    return (
      <div className="detail-section">
        <h4>Agent Config</h4>
        <p className="detail-inline-empty"><Spinner label="Loading…" /></p>
      </div>
    );
  }

  return (
    <div className="detail-section agent-config-section">
      <div className="agent-config-header">
        <h4>{filename}</h4>
        <div className="agent-config-actions">
          {saveMsg && (
            <span style={{ fontSize: "11px", color: "var(--success)" }}>
              {saveMsg}
            </span>
          )}
          <button
            className="agent-config-save-btn"
            onClick={handleSave}
            disabled={saving || !isDirty}
            aria-busy={saving}
            title="Save"
          >
            {saving ? <Spinner size={12} /> : "Save"}
          </button>
        </div>
      </div>
      {error && (
        <p
          style={{
            fontSize: "11px",
            color: "var(--danger, #ef4444)",
            marginBottom: "8px",
          }}
        >
          {error}
        </p>
      )}
      {format === "raw" ? (
        <>
          <p className="detail-inline-empty">
            Existing Harness instructions are shown as-is and are not converted to the Choruz template.
          </p>
          <textarea
            className="agent-config-editor"
            value={rawContent}
            onChange={(event) => setRawContent(event.target.value)}
            aria-label={`${filename} contents`}
            spellCheck={false}
          />
        </>
      ) : (
        <AgentInstructionForm fields={fields} onChange={setFields} />
      )}
      <p
        style={{
          fontSize: "10px",
          color: "var(--text-muted)",
          marginTop: "6px",
        }}
      >
        {binding.workspace_path}/{filename}
      </p>
    </div>
  );
}
