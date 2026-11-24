-- Optional filesystem location of the project's workspace.
-- NULL means the project is not bound to a directory yet; ACP flows that
-- operate on the filesystem require it, everything else keeps working.

ALTER TABLE projects ADD COLUMN working_dir TEXT;
