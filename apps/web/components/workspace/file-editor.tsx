"use client";
import { lazy, Suspense, useState, useEffect, useCallback, useRef } from "react";
import { trace } from "../../lib/api/choruz-trace";
import { Spinner } from "../ui/spinner";
import { EditorView, keymap, lineNumbers, highlightActiveLine, highlightActiveLineGutter, drawSelection } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { defaultKeymap, indentWithTab, history, historyKeymap } from "@codemirror/commands";
import { syntaxHighlighting, HighlightStyle, bracketMatching, foldGutter } from "@codemirror/language";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { tags } from "@lezer/highlight";
import { transportFetch } from "../../lib/api/transport";

// Lazy-load the markdown renderer. Same pattern as message-bubble.tsx so
// the bundle only pulls react-markdown when a Markdown file is previewed.
const ReactMarkdown = lazy(() => import("react-markdown"));
const remarkGfmModule = import("remark-gfm").then((m) => m.default);
let _remarkGfm: typeof import("remark-gfm").default | null = null;
remarkGfmModule.then((mod) => { _remarkGfm = mod; });

const MARKDOWN_EXTS = new Set(["md", "mdx", "markdown"]);

// Custom dark theme matching Choruz's #0a0a0a background
const choruzTheme = EditorView.theme({
  "&": {
    backgroundColor: "var(--bg, #0a0a0a)",
    color: "var(--text, #e4e4e7)",
  },
  ".cm-content": { caretColor: "var(--accent, #06a590)" },
  ".cm-cursor": { borderLeftColor: "var(--accent, #06a590)" },
  ".cm-activeLine": { backgroundColor: "rgba(255,255,255,0.03)" },
  ".cm-activeLineGutter": { backgroundColor: "rgba(255,255,255,0.03)" },
  ".cm-gutters": {
    backgroundColor: "var(--bg, #0a0a0a)",
    color: "rgba(255,255,255,0.2)",
    borderRight: "1px solid var(--border, #222)",
  },
  ".cm-lineNumbers .cm-gutterElement": { color: "rgba(255,255,255,0.2)" },
  ".cm-foldGutter .cm-gutterElement": { color: "rgba(255,255,255,0.15)" },
  ".cm-selectionBackground, &.cm-focused .cm-selectionBackground": {
    backgroundColor: "rgba(16,185,129,0.15) !important",
  },
  ".cm-matchingBracket": { backgroundColor: "rgba(16,185,129,0.2)", color: "inherit !important" },
  ".cm-searchMatch": { backgroundColor: "rgba(234,179,8,0.25)" },
  ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "rgba(234,179,8,0.4)" },
  ".cm-selectionMatch": { backgroundColor: "rgba(16,185,129,0.1)" },
}, { dark: true });

const choruzHighlight = HighlightStyle.define([
  { tag: tags.keyword, color: "#c792ea" },
  { tag: tags.operator, color: "#89ddff" },
  { tag: tags.string, color: "#c3e88d" },
  { tag: tags.number, color: "#f78c6c" },
  { tag: tags.bool, color: "#ff5874" },
  { tag: tags.comment, color: "#546e7a", fontStyle: "italic" },
  { tag: tags.function(tags.variableName), color: "#82aaff" },
  { tag: tags.typeName, color: "#ffcb6b" },
  { tag: tags.className, color: "#ffcb6b" },
  { tag: tags.propertyName, color: "#f07178" },
  { tag: tags.definition(tags.variableName), color: "#82aaff" },
  { tag: tags.variableName, color: "#e4e4e7" },
  { tag: tags.heading, color: "#c792ea", fontWeight: "bold" },
  { tag: tags.link, color: "#06a590", textDecoration: "underline" },
  { tag: tags.meta, color: "#546e7a" },
  { tag: tags.tagName, color: "#f07178" },
  { tag: tags.attributeName, color: "#ffcb6b" },
  { tag: tags.attributeValue, color: "#c3e88d" },
]);

// Language imports (lazy)
const LANG_LOADERS: Record<string, () => Promise<{ default: any } | any>> = {
  js: () => import("@codemirror/lang-javascript").then(m => m.javascript()),
  jsx: () => import("@codemirror/lang-javascript").then(m => m.javascript({ jsx: true })),
  ts: () => import("@codemirror/lang-javascript").then(m => m.javascript({ typescript: true })),
  tsx: () => import("@codemirror/lang-javascript").then(m => m.javascript({ jsx: true, typescript: true })),
  json: () => import("@codemirror/lang-json").then(m => m.json()),
  md: () => import("@codemirror/lang-markdown").then(m => m.markdown()),
  html: () => import("@codemirror/lang-html").then(m => m.html()),
  css: () => import("@codemirror/lang-css").then(m => m.css()),
  py: () => import("@codemirror/lang-python").then(m => m.python()),
  rs: () => import("@codemirror/lang-rust").then(m => m.rust()),
  toml: () => import("@codemirror/lang-json").then(m => m.json()), // fallback
  yaml: () => import("@codemirror/lang-json").then(m => m.json()), // fallback
  yml: () => import("@codemirror/lang-json").then(m => m.json()),
};

type FileEditorProps = {
  filePath: string;
  workspaceId?: string | null;
  sessionToken: string;
  onClose: () => void;
  onDirty: (dirty: boolean) => void;
};

export function FileEditor({ filePath, workspaceId, sessionToken, onClose, onDirty }: FileEditorProps) {
  const [content, setContent] = useState<string | null>(null);
  const [originalContent, setOriginalContent] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [mode, setMode] = useState<"edit" | "preview">("edit");
  const editorRef = useRef<HTMLDivElement>(null);
  const viewRef = useRef<EditorView | null>(null);
  const contentRef = useRef<string | null>(null);

  const ext = filePath.split('.').pop()?.toLowerCase() || '';
  const isMarkdown = MARKDOWN_EXTS.has(ext);

  // Reset mode to edit when switching to a non-markdown file
  useEffect(() => {
    if (!isMarkdown && mode === "preview") setMode("edit");
  }, [isMarkdown, mode]);

  const togglePreview = useCallback(() => {
    if (!isMarkdown) return;
    setMode((m) => (m === "edit" ? "preview" : "edit"));
  }, [isMarkdown]);

  // Cmd/Ctrl + Shift + V — toggle markdown preview. Cmd+Shift+V is also
  // "Paste as Plain Text" in Chrome but only fires inside text inputs; our
  // editor is a contentEditable so we preventDefault to keep the shortcut.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!isMarkdown) return;
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.shiftKey && (e.key === "v" || e.key === "V")) {
        e.preventDefault();
        togglePreview();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [isMarkdown, togglePreview]);

  // Track dirty state
  const isDirty = content !== originalContent;
  const onDirtyRef = useRef(onDirty);
  onDirtyRef.current = onDirty;
  useEffect(() => { onDirtyRef.current(isDirty); }, [isDirty]);

  // Load file content
  useEffect(() => {
    setContent(null);
    setOriginalContent(null);
    setError(null);
    const params = new URLSearchParams({ action: "read", path: filePath });
    if (workspaceId) params.set("workspace_id", workspaceId);
    transportFetch(`/api/filesystem?${params.toString()}`)
      .then(r => r.json())
      .then(data => {
        if (data.error) setError(typeof data.error === "string" ? data.error : JSON.stringify(data.error));
        else { setContent(data.content); setOriginalContent(data.content); contentRef.current = data.content; }
      })
      .catch(e => setError(e.message));
  }, [filePath, workspaceId]);

  // Save handler
  const handleSave = useCallback(async () => {
    const cur = contentRef.current;
    if (cur === originalContent || saving || cur === null) return;
    setSaving(true);
    const span = trace.start("file_save", { path: filePath });
    try {
      const res = await fetch('/api/filesystem', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: filePath, content: cur, ...(workspaceId ? { workspace_id: workspaceId } : {}) }),
      });
      const data = await res.json();
      if (data.error) {
        const errMsg = typeof data.error === "string" ? data.error : JSON.stringify(data.error);
        span.end({ error: errMsg });
        setError(errMsg);
      } else {
        span.end({ status: "ok", size: cur?.length });
        setOriginalContent(cur);
        setError(null);
      }
    } catch (e) {
      span.end({ error: String(e) });
      setError(String(e));
    }
    setSaving(false);
  }, [originalContent, saving, filePath, workspaceId]);

  // Initialize CodeMirror when content is loaded (skip in preview mode — the
  // editor DOM target isn't mounted and there's nothing to render.)
  useEffect(() => {
    if (content === null || mode === "preview" || !editorRef.current) return;
    if (viewRef.current) {
      viewRef.current.destroy();
      viewRef.current = null;
    }

    const saveRef = { current: handleSave };

    const extensions = [
      lineNumbers(),
      highlightActiveLine(),
      highlightActiveLineGutter(),
      drawSelection(),
      bracketMatching(),
      foldGutter(),
      history(),
      highlightSelectionMatches(),
      choruzTheme,
      syntaxHighlighting(choruzHighlight, { fallback: true }),
      keymap.of([
        ...defaultKeymap,
        ...historyKeymap,
        ...searchKeymap,
        indentWithTab,
        { key: "Mod-s", run: () => { saveRef.current(); return true; } },
      ]),
      EditorView.updateListener.of((update) => {
        if (update.docChanged) {
          const newContent = update.state.doc.toString();
          contentRef.current = newContent;
          setContent(newContent);
        }
      }),
      EditorView.theme({
        "&": { height: "100%", fontSize: "13px" },
        ".cm-scroller": { overflow: "auto", fontFamily: "'SF Mono', 'Fira Code', 'JetBrains Mono', monospace" },
        ".cm-gutters": { borderRight: "1px solid var(--border, #333)" },
      }),
    ];

    // Load language extension
    const langLoader = LANG_LOADERS[ext];
    const initEditor = (langExt?: any) => {
      if (!editorRef.current) return;
      const state = EditorState.create({
        doc: content,
        extensions: langExt ? [...extensions, langExt] : extensions,
      });
      viewRef.current = new EditorView({
        state,
        parent: editorRef.current,
      });
    };

    if (langLoader) {
      langLoader().then(lang => initEditor(lang)).catch(() => initEditor());
    } else {
      initEditor();
    }

    return () => {
      viewRef.current?.destroy();
      viewRef.current = null;
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [content === null ? null : filePath, mode]); // reinit when file changes OR when switching back from preview

  // Update save handler ref
  useEffect(() => {
    // Keep the save keymap up to date — no need to rebuild the editor
  }, [handleSave]);

  if (error) return (
    <div className="file-editor">
      <div className="file-editor-toolbar">
        <span className="file-editor-path">{filePath}</span>
        <div className="file-editor-actions">
          <button onClick={onClose} className="file-editor-save-btn">Close</button>
        </div>
      </div>
      <div className="file-editor-error">{error}</div>
    </div>
  );

  if (content === null) return (
    <div className="file-editor">
      <div className="file-editor-toolbar">
        <span className="file-editor-path">{filePath}</span>
      </div>
      <div className="file-editor-loading"><Spinner label="Loading…" /></div>
    </div>
  );

  return (
    <div className="file-editor">
      <div className="file-editor-toolbar">
        <span className="file-editor-path">{filePath}</span>
        <div className="file-editor-actions">
          {isDirty && <span className="file-editor-dirty">Modified</span>}
          <button onClick={handleSave} disabled={!isDirty || saving} className="file-editor-save-btn">
            {saving ? 'Saving…' : 'Save'}
          </button>
        </div>
      </div>
      {mode === "preview" ? (
        <div className="file-editor-preview">
          <Suspense fallback={<div className="file-editor-loading"><Spinner label="Loading preview…" /></div>}>
            <ReactMarkdown remarkPlugins={_remarkGfm ? [_remarkGfm] : []}>
              {content ?? ""}
            </ReactMarkdown>
          </Suspense>
        </div>
      ) : (
        <div ref={editorRef} className="file-editor-cm" />
      )}
    </div>
  );
}
