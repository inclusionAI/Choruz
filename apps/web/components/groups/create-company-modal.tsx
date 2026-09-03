"use client";

import { useState } from "react";
import { trace } from "../../lib/api/choruz-trace";
import { FolderPickerModal } from "../workspace/folder-picker-modal";
import { buildManagerInstructions } from "../../lib/agents/ai-manager-instructions";
import type { Company } from "../../lib/api/choruz-types";
import type { DriverId } from "../../lib/groups/team-templates";
import { LOCAL_TERMINAL_DRIVER_IDS } from "../../lib/drivers/driver-registry";
import { DriverSelect } from "../agents/driver-select";
import { Modal } from "../ui/modal";
import { DriverModelPicker } from "../agents/driver-model-picker";
import { transportFetch } from "../../lib/api/transport";

type Props = {
  principalId: string;
  sessionToken: string;
  onClose: () => void;
  onCreated: (company: Company) => void;
};

export function CreateCompanyModal({ principalId, sessionToken, onClose, onCreated }: Props) {
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [folderPath, setFolderPath] = useState("");
  const [showFolderPicker, setShowFolderPicker] = useState(false);
  const [withManager, setWithManager] = useState(true);
  const [managerDriver, setManagerDriver] = useState<DriverId>("claude_terminal");
  const [managerModel, setManagerModel] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [createdCompany, setCreatedCompany] = useState<Company | null>(null);

  const provisionManager = async (company: Company): Promise<boolean> => {
    const workspacePath = company.folder_path?.trim() || null;
    try {
      const mgrResp = await transportFetch("/api/agents/provision", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          name: "AI Manager",
          driver_type: managerDriver,
          ...(managerModel.trim() ? { model: managerModel.trim() } : {}),
          idempotency_key: `company:${company.id}:ai-manager`,
          instructions: buildManagerInstructions(
            company.name,
            workspacePath,
          ),
          workspace_id: company.id,
          ...(workspacePath ? { workspace_path: workspacePath } : {}),
        }),
      });
      if (mgrResp.ok) return true;
      const mgrBody = await mgrResp.json().catch(() => ({}));
      console.error("AI Manager provision failed:", mgrResp.status, mgrBody);
      setError(
        `Company created, but AI Manager failed: ${mgrBody.error?.detail || mgrBody.error || `HTTP ${mgrResp.status}`}`,
      );
    } catch (mgrErr) {
      console.error("AI Manager provision error:", mgrErr);
      setError("Company created, but AI Manager failed to provision.");
    }
    return false;
  };

  const handleCreate = async () => {
    if (!name.trim()) return;
    setLoading(true);
    setError(null);
    const span = trace.start("create_company", { name: name.trim(), withManager });
    try {
      if (createdCompany) {
        if (await provisionManager(createdCompany)) {
          span.end({ status: 200, retriedManager: true });
          onCreated(createdCompany);
        } else {
          span.end({ status: 207, retriedManager: true });
        }
        return;
      }
      const resp = await transportFetch("/api/companies", {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          actor_id: principalId,
          name: name.trim(),
          description: description.trim() || undefined,
          folder_path: folderPath.trim() || undefined,
        }),
      });
      if (!resp.ok) {
        const body = await resp.json().catch(() => ({}));
        throw new Error(body.error?.detail || body.error || `HTTP ${resp.status}`);
      }
      const company = await resp.json();
      setCreatedCompany(company);

      // Auto-provision AI Manager if requested
      if (withManager && !(await provisionManager(company))) {
        span.end({ status: 207 });
        return;
      }

      span.end({ status: 200 });
      onCreated(company);
    } catch (err) {
      span.end({ error: err instanceof Error ? err.message : String(err) });
      setError(err instanceof Error ? err.message : "Failed to create company");
    } finally {
      setLoading(false);
    }
  };

  return (
    <>
      <Modal title="Create Company" onClose={onClose} describedBy={error ? "create-company-error" : undefined}>
        <div className="modal-form">
          <label>
            Company Name
            <input
              type="text"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Acme Corp"
              autoFocus
              required
              disabled={Boolean(createdCompany)}
            />
          </label>
          <label>
            Description (optional)
            <input
              type="text"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              placeholder="What is this company for?"
            />
          </label>

          {/* Folder picker */}
          <label>
            Workspace Folder
            <div className="folder-select-row">
              <input
                type="text"
                value={folderPath}
                onChange={(e) => setFolderPath(e.target.value)}
                placeholder="Paste a path or click Browse…"
                className="folder-path-display"
                spellCheck={false}
                autoCapitalize="off"
                autoCorrect="off"
              />
              <button
                type="button"
                className="folder-browse-btn"
                onClick={() => setShowFolderPicker(true)}
              >
                <svg width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" style={{ marginRight: 4 }}>
                  <path d="M2 4.5C2 3.67 2.67 3 3.5 3h2.59a1 1 0 01.7.29L8 4.5h4.5c.83 0 1.5.67 1.5 1.5v5.5c0 .83-.67 1.5-1.5 1.5h-9A1.5 1.5 0 012 11.5V4.5z"/>
                </svg>
                Browse
              </button>
            </div>
            {folderPath && (
              <span style={{ fontSize: "var(--fs-xs)", color: "var(--text-muted)", marginTop: 2 }}>
                Agents will work inside this folder. A file tree will appear in the sidebar.
              </span>
            )}
          </label>

          {/* AI Manager option */}
          <label
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--space-2)",
              cursor: "pointer",
              userSelect: "none",
            }}
          >
            <input
              type="checkbox"
              checked={withManager}
              onChange={(e) => setWithManager(e.target.checked)}
              disabled={Boolean(createdCompany)}
              style={{ width: "auto", margin: 0 }}
            />
            Include AI Manager
          </label>
          {withManager && (
            <label>
              Manager Driver
              <DriverSelect
                value={managerDriver}
                disabled={Boolean(createdCompany)}
                onChange={(driver) => {
                  setManagerDriver(driver);
                  setManagerModel("");
                }}
                drivers={LOCAL_TERMINAL_DRIVER_IDS}
              />
            </label>
          )}
          {withManager && (
            <DriverModelPicker
              driver={managerDriver}
              model={managerModel}
              onChange={setManagerModel}
              label="Manager Model"
              disabled={Boolean(createdCompany)}
            />
          )}

          {error && <p id="create-company-error" className="modal-form-error" role="alert" aria-live="polite">{error}</p>}
        </div>
        <div className="modal-actions">
          <button
            className="btn-cancel"
            onClick={() => createdCompany ? onCreated(createdCompany) : onClose()}
            disabled={loading}
          >
            {createdCompany ? "Continue without AI Manager" : "Cancel"}
          </button>
          <button
            className="btn-primary"
            onClick={handleCreate}
            disabled={loading || !name.trim()}
          >
            {loading ? (createdCompany ? "Retrying…" : "Creating…") : (createdCompany ? "Retry AI Manager" : "Create")}
          </button>
        </div>
      </Modal>

      {/* Nested folder picker modal */}
      {showFolderPicker && (
        <FolderPickerModal
          initialPath={folderPath || undefined}
          onSelect={(path) => {
            setFolderPath(path);
            setShowFolderPicker(false);
          }}
          onClose={() => setShowFolderPicker(false)}
        />
      )}
    </>
  );
}
