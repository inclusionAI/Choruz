-- Add folder_path column to company table for VS Code-style "Open Folder" workspace support
ALTER TABLE company ADD COLUMN IF NOT EXISTS folder_path TEXT;
