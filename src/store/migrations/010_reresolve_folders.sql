DELETE FROM directories
WHERE remote_url IS NULL
  AND repo_path = (SELECT text FROM labels WHERE labels.id = directories.cwd_label);
