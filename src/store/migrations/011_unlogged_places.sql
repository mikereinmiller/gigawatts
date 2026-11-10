ALTER TABLE not_in_transcript ADD COLUMN cwd_label INTEGER REFERENCES labels (id);
ALTER TABLE not_in_transcript ADD COLUMN branch_label INTEGER REFERENCES labels (id);

UPDATE not_in_transcript SET (cwd_label, branch_label) = (
    SELECT cwd_label, coalesce(head_branch_label, git_branch_label) FROM requests
    WHERE session_id = not_in_transcript.session_id
    GROUP BY 1, 2 ORDER BY count(*) DESC LIMIT 1);
