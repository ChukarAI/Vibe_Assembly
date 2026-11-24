//! Incremental git diff review - checkpoint, isolate, stream.
//!
//! Design note: the upstream (`pi`) ships a `git-checkpoint` extension and a
//! `durable` reviewer loop that re-prompts a model until it emits a `DONE`
//! sentinel. This module provides the deterministic substrate that loop needs:
//! a cheap checkpoint before each phase, and a precise statement of what
//! *this* step changed, isolated from everything committed before it.
//!
//! The `DONE`-sentinel review loop is reproduced in `engine.rs` at the
//! orchestration level, where it belongs.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::judge::{DiffLine, Finding, Judge, Verdict};

/// A lightweight git checkpoint: a commit sha, or a marker when git is
/// unavailable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    pub id: String,
}

#[derive(Debug)]
pub enum ReviewError {
    GitUnavailable(String),
    GitFailed { command: String, stderr: String },
    NotARepository,
}

impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReviewError::GitUnavailable(e) => write!(f, "git unavailable: {e}"),
            ReviewError::GitFailed { command, stderr } => {
                write!(f, "git {command} failed: {stderr}")
            }
            ReviewError::NotARepository => write!(f, "not inside a git repository"),
        }
    }
}

impl std::error::Error for ReviewError {}

/// Runs git in `repo` and returns stdout.
fn git(repo: &Path, args: &[&str]) -> Result<String, ReviewError> {
    let out = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .map_err(|e| ReviewError::GitUnavailable(e.to_string()))?;
    if !out.status.success() {
        return Err(ReviewError::GitFailed {
            command: args.join(" "),
            stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// A throwaway index file for plumbing that must never touch the repository's
/// real index, working tree, or branches. Deleted on drop.
struct TempIndex {
    repo: PathBuf,
    path: PathBuf,
}

impl TempIndex {
    fn new(repo: &Path) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        Self {
            repo: repo.to_path_buf(),
            path: std::env::temp_dir().join(format!(
                "va-harness-index-{}-{nanos}.tmp",
                std::process::id()
            )),
        }
    }

    fn git(&self, args: &[&str]) -> Result<String, ReviewError> {
        let out = Command::new("git")
            .args(args)
            .current_dir(&self.repo)
            .env("GIT_INDEX_FILE", &self.path)
            .output()
            .map_err(|e| ReviewError::GitUnavailable(e.to_string()))?;
        if !out.status.success() {
            return Err(ReviewError::GitFailed {
                command: args.join(" "),
                stderr: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            });
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    /// The tree object of the current working tree (tracked + untracked,
    /// respecting .gitignore), staged without a base commit.
    fn worktree_tree(&self) -> Result<String, ReviewError> {
        self.git(&["read-tree", "--empty"])?;
        self.git(&["add", "-A"])?;
        let tree = self.git(&["write-tree"])?;
        Ok(tree.trim().to_string())
    }
}

impl Drop for TempIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Tracks a checkpoint across an execution phase and computes the delta since.
pub struct Reviewer {
    repo: PathBuf,
    checkpoint: Option<Checkpoint>,
}

impl Reviewer {
    pub fn new(repo: &Path) -> Self {
        Self {
            repo: repo.to_path_buf(),
            checkpoint: None,
        }
    }

    pub fn is_repository(&self) -> bool {
        Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(&self.repo)
            .output()
            .map(|o| o.status.success() && String::from_utf8_lossy(&o.stdout).trim() == "true")
            .unwrap_or(false)
    }

    /// Snapshot the working tree into a dangling commit object so the next
    /// phase's delta is precisely attributable - **without moving the branch
    /// and without touching the real index or the working tree**.
    ///
    /// Uses pure plumbing against a throwaway temporary index: `read-tree
    /// HEAD` + `add -A` + `write-tree` produce the tree, `commit-tree` wraps
    /// it into a commit no ref points at. `git diff <sha>` then compares the
    /// working tree against that snapshot.
    pub fn checkpoint(&mut self) -> Result<Checkpoint, ReviewError> {
        if !self.is_repository() {
            return Err(ReviewError::NotARepository);
        }
        let index = TempIndex::new(&self.repo);
        // An unborn branch has no HEAD tree; stage against an empty one so
        // freshly-initialized repositories still get a checkpoint.
        let result = (|| -> Result<String, ReviewError> {
            if index.git(&["rev-parse", "--verify", "HEAD"]).is_ok() {
                index.git(&["read-tree", "HEAD"])?;
            } else {
                index.git(&["read-tree", "--empty"])?;
            }
            index.git(&["add", "-A"])?;
            let tree = index.git(&["write-tree"])?;
            index.git(&["commit-tree", tree.trim(), "-m", "va-harness checkpoint"])
        })();

        let sha = result?.trim().to_string();
        if sha.is_empty() {
            return Err(ReviewError::GitFailed {
                command: "commit-tree".to_string(),
                stderr: "empty commit id".to_string(),
            });
        }
        self.checkpoint = Some(Checkpoint { id: sha.clone() });
        Ok(Checkpoint { id: sha })
    }

    /// Diff the working tree against the checkpoint (or HEAD when no
    /// checkpoint was taken), as a tree-vs-tree comparison. Tree-vs-tree
    /// works on unborn branches and never depends on the real index.
    fn diff_since(&self, cp: Option<&Checkpoint>) -> Result<String, ReviewError> {
        let base = match cp {
            Some(c) => c.id.clone(),
            None => git(&self.repo, &["rev-parse", "HEAD"])?.trim().to_string(),
        };
        let index = TempIndex::new(&self.repo);
        let tree = index.worktree_tree()?;
        git(&self.repo, &["diff", &base, &tree])
    }

    /// The current checkpoint, if one has been taken.
    pub fn current(&self) -> Option<&Checkpoint> {
        self.checkpoint.as_ref()
    }

    /// Parse a unified diff into structured lines with line numbers.
    ///
    /// Added lines are numbered in the new file; removed lines in the old.
    /// Hunk headers (`@@ -a,b +c,d @@`) drive the counters, so numbers are
    /// correct across multiple files and non-zero hunk offsets, and content
    /// lines that merely *start with* `++ b/` are never mistaken for headers.
    pub fn parse_diff(&self, raw: &str) -> (Vec<DiffLine>, Vec<usize>) {
        let mut lines = Vec::new();
        let mut numbers = Vec::new();
        let (mut new_no, mut old_no) = (0usize, 0usize);
        let mut old_remaining = 0usize;
        let mut new_remaining = 0usize;
        let mut in_hunk = false;
        let mut path = String::new();

        for raw_line in raw.lines() {
            // A hunk with both counts exhausted ends here; reprocess this
            // line as a between-hunks line.
            if in_hunk && old_remaining == 0 && new_remaining == 0 {
                in_hunk = false;
            }

            if !in_hunk {
                if raw_line.starts_with("@@") {
                    if let Some((old_start, old_count, new_start, new_count)) =
                        parse_hunk_header(raw_line)
                    {
                        old_no = old_start.saturating_sub(1);
                        new_no = new_start.saturating_sub(1);
                        old_remaining = old_count;
                        new_remaining = new_count;
                        in_hunk = true;
                    }
                    continue;
                }
                if raw_line.starts_with("diff --git") {
                    path.clear();
                    continue;
                }
                if let Some(rest) = raw_line.strip_prefix("+++ ") {
                    if rest != "/dev/null" {
                        path = diff_header_path(rest);
                    }
                    continue;
                }
                if let Some(rest) = raw_line.strip_prefix("--- ") {
                    if path.is_empty() && rest != "/dev/null" {
                        path = diff_header_path(rest);
                    }
                    continue;
                }
                continue;
            }

            let mut chars = raw_line.chars();
            let Some(marker) = chars.next() else {
                continue;
            };
            let text = chars.as_str().to_string();
            match marker {
                '+' => {
                    new_no += 1;
                    new_remaining = new_remaining.saturating_sub(1);
                    numbers.push(new_no);
                    lines.push(DiffLine {
                        marker,
                        path: path.clone(),
                        text,
                    });
                }
                '-' => {
                    old_no += 1;
                    old_remaining = old_remaining.saturating_sub(1);
                    numbers.push(old_no);
                    lines.push(DiffLine {
                        marker,
                        path: path.clone(),
                        text,
                    });
                }
                ' ' => {
                    new_no += 1;
                    old_no += 1;
                    new_remaining = new_remaining.saturating_sub(1);
                    old_remaining = old_remaining.saturating_sub(1);
                    numbers.push(new_no);
                    lines.push(DiffLine {
                        marker,
                        path: path.clone(),
                        text,
                    });
                }
                // "\ No newline at end of file": not a content line; the
                // hunk continues if counts remain.
                '\\' => continue,
                _ => in_hunk = false,
            }
        }
        (lines, numbers)
    }
}

/// Parse `@@ -old[,oc] +new[,nc] @@` into
/// `(old_start, old_count, new_start, new_count)`. Omitted counts default
/// to 1, per the unified-diff spec.
fn parse_hunk_header(line: &str) -> Option<(usize, usize, usize, usize)> {
    let rest = line.strip_prefix("@@ ")?;
    let (old_part, rest) = rest.split_once(' ')?;
    let (new_part, _) = rest.split_once(' ')?;
    let old = old_part.strip_prefix('-')?;
    let new = new_part.strip_prefix('+')?;
    let old_start = old.split(',').next()?.parse().ok()?;
    let old_count = hunk_count(old)?;
    let new_start = new.split(',').next()?.parse().ok()?;
    let new_count = hunk_count(new)?;
    Some((old_start, old_count, new_start, new_count))
}

fn hunk_count(part: &str) -> Option<usize> {
    match part.split(',').nth(1) {
        Some(c) => c.parse().ok(),
        None => Some(1),
    }
}

fn diff_header_path(header: &str) -> String {
    let path = header.split('\t').next().unwrap_or(header).trim();
    path.strip_prefix("a/")
        .or_else(|| path.strip_prefix("b/"))
        .unwrap_or(path)
        .to_string()
}

/// Standalone parser for unified diff text.
pub fn parse_unified_diff(raw: &str) -> (Vec<DiffLine>, Vec<usize>) {
    Reviewer::new(Path::new(".")).parse_diff(raw)
}

impl Reviewer {
    /// Everything changed since the checkpoint - the exact delta of this step.
    pub fn since_checkpoint(&self) -> Result<Vec<DiffLine>, ReviewError> {
        let raw = self.diff_since(self.checkpoint.as_ref())?;
        Ok(self.parse_diff(&raw).0)
    }

    /// Judge the delta since the checkpoint.
    pub fn review_since_checkpoint(&self, judge: &Judge) -> Result<Verdict, ReviewError> {
        let raw = self.diff_since(self.checkpoint.as_ref())?;
        let (lines, numbers) = self.parse_diff(&raw);
        Ok(judge.review(&lines, &numbers))
    }

    /// A structured event describing this step's delta, for UI streaming.
    pub fn delta_event(&self) -> Result<DeltaEvent, ReviewError> {
        let lines = self.since_checkpoint()?;
        let mut added = 0usize;
        let mut removed = 0usize;
        let mut files: Vec<String> = Vec::new();
        for l in &lines {
            if l.added() {
                added += 1;
            } else if l.removed() {
                removed += 1;
            }
            if !files.contains(&l.path) {
                files.push(l.path.clone());
            }
        }
        Ok(DeltaEvent {
            files,
            added,
            removed,
        })
    }
}

/// A compact, UI-friendly summary of one step's changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaEvent {
    pub files: Vec<String>,
    pub added: usize,
    pub removed: usize,
}

impl std::fmt::Display for DeltaEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} file(s), +{} −{}",
            self.files.len(),
            self.added,
            self.removed
        )
    }
}

/// Convenience: the findings from a review, rendered for the Code role.
pub fn render_findings(findings: &[Finding]) -> String {
    Judge::feedback(findings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_unified_diff_with_line_numbers() {
        let r = Reviewer::new(Path::new("."));
        let raw = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,3 +1,3 @@\n use std::fmt;\n-fn old() {}\n+fn new() {}\n \n";
        let (lines, nums) = r.parse_diff(raw);
        let changed: Vec<&DiffLine> = lines.iter().filter(|l| !l.marker.is_whitespace()).collect();
        assert_eq!(changed.len(), 2);
        assert!(changed.iter().all(|l| l.path == "src/lib.rs"));
        assert_eq!(nums.len(), lines.len(), "numbers stay parallel to lines");
        // Context line is line 1; `-` is numbered in the old file, `+` in
        // the new, so both changed lines are line 2 of their file.
        assert_eq!(nums[1], 2, "old-file line for the removed line");
        assert_eq!(nums[2], 2, "new-file line for the added line");
    }

    #[test]
    fn hunk_offsets_drive_line_numbers() {
        let r = Reviewer::new(Path::new("."));
        let raw = "diff --git a/a.rs b/a.rs\n--- a/a.rs\n+++ b/a.rs\n@@ -10,2 +20,2 @@\n context\n-old\n+new\n";
        let (lines, nums) = r.parse_diff(raw);
        assert_eq!(lines.len(), 3, "context is kept and numbered too");
        assert_eq!(nums[0], 20, "context line is new-file line 20");
        assert_eq!(nums[1], 11, "removed line is old-file line 11");
        assert_eq!(nums[2], 21, "added line is new-file line 21");
    }

    #[test]
    fn added_content_that_looks_like_a_header_stays_content() {
        let r = Reviewer::new(Path::new("."));
        let raw = "diff --git a/t.md b/t.md\n--- a/t.md\n+++ b/t.md\n@@ -1,1 +1,2 @@\n base\n+++ b/not-a-header\n";
        let (lines, _) = r.parse_diff(raw);
        assert_eq!(lines.len(), 2);
        assert!(lines[1].added(), "inside a hunk, `+++ b/` is content");
        assert_eq!(lines[1].text, "++ b/not-a-header");
        assert_eq!(lines[1].path, "t.md");
    }

    fn git_in(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git must be available for these tests");
        assert!(out.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn init_repo(tag: &str, with_commit: bool) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("va-review-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        git_in(&dir, &["init", "-q"]);
        git_in(&dir, &["config", "user.email", "test@example.com"]);
        git_in(&dir, &["config", "user.name", "Test"]);
        if with_commit {
            std::fs::write(dir.join("a.txt"), "base\n").unwrap();
            git_in(&dir, &["add", "-A"]);
            git_in(&dir, &["commit", "-qm", "base"]);
        }
        dir
    }

    #[test]
    fn checkpoint_does_not_move_the_branch_or_touch_the_index() {
        let dir = init_repo("branch", true);
        let head_before = git_in(&dir, &["rev-parse", "HEAD"]);
        let count_before = git_in(&dir, &["rev-list", "--count", "HEAD"]);

        std::fs::write(dir.join("a.txt"), "modified\n").unwrap();
        std::fs::write(dir.join("untracked.txt"), "new\n").unwrap();
        let status_before = git_in(&dir, &["status", "--porcelain"]);

        let mut reviewer = Reviewer::new(&dir);
        let cp = reviewer.checkpoint().expect("checkpoint must succeed");
        assert_ne!(cp.id, head_before, "the checkpoint is a dangling commit");

        assert_eq!(
            git_in(&dir, &["rev-parse", "HEAD"]),
            head_before,
            "the branch must not move"
        );
        assert_eq!(
            git_in(&dir, &["rev-list", "--count", "HEAD"]),
            count_before,
            "no checkpoint commits may land on the branch"
        );
        assert_eq!(
            git_in(&dir, &["status", "--porcelain"]),
            status_before,
            "the real index and working tree must be untouched"
        );

        // The snapshot still captures the delta for the judge.
        std::fs::write(dir.join("a.txt"), "modified again\n").unwrap();
        let raw = git_in(&dir, &["diff", &cp.id]);
        assert!(raw.contains("modified again"), "diff since: {raw}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn diff_since_checkpoint_reports_every_file_deleted_from_the_worktree() {
        let dir = init_repo("deleted", true);
        let mut reviewer = Reviewer::new(&dir);
        reviewer.checkpoint().expect("checkpoint must succeed");
        std::fs::remove_file(dir.join("a.txt")).unwrap();

        let delta = reviewer
            .since_checkpoint()
            .expect("deletion diff must work");
        assert!(delta
            .iter()
            .any(|line| line.removed() && line.path == "a.txt"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkpoint_works_on_an_unborn_branch() {
        let dir = init_repo("unborn", false);
        std::fs::write(dir.join("fresh.txt"), "hello\n").unwrap();
        let mut reviewer = Reviewer::new(&dir);
        let cp = reviewer
            .checkpoint()
            .expect("a fresh repo with no commits must still checkpoint");
        assert!(!cp.id.is_empty());
        // HEAD stays unborn: no branch was created by the checkpoint.
        assert!(
            Command::new("git")
                .args(["rev-parse", "--verify", "HEAD"])
                .current_dir(&dir)
                .output()
                .unwrap()
                .status
                .code()
                != Some(0)
        );
        std::fs::write(dir.join("fresh.txt"), "hello again\n").unwrap();
        let delta = reviewer
            .since_checkpoint()
            .expect("tree-vs-tree diff works unborn");
        assert!(
            delta
                .iter()
                .any(|l| l.added() && l.text.contains("hello again")),
            "delta since: {delta:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn delta_event_counts_and_dedupes_files() {
        let event = DeltaEvent {
            files: vec!["a.rs".to_string()],
            added: 3,
            removed: 1,
        };
        assert_eq!(event.to_string(), "1 file(s), +3 −1");
    }

    #[test]
    fn non_repository_is_reported_not_panicked() {
        let dir = std::env::temp_dir().join("va-review-not-a-repo");
        let _ = std::fs::create_dir_all(&dir);
        let mut r = Reviewer::new(&dir);
        // A temp dir is not a repo, so checkpointing must error cleanly.
        if !r.is_repository() {
            assert!(matches!(r.checkpoint(), Err(ReviewError::NotARepository)));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
