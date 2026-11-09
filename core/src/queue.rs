use crate::model::{Job, JobTemplate};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepStatus {
    Pending,
    Running,
    Finished,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TranscribeJob {
    pub audio_file: PathBuf,
    pub output_file: PathBuf,
    pub model: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default = "default_device")]
    pub device: String,
    #[serde(default = "default_compute_type")]
    pub compute_type: String,
    #[serde(default)]
    pub vad_filter: bool,
}

fn default_device() -> String {
    "cpu".to_string()
}

fn default_compute_type() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "job")]
pub enum StepKind {
    Ffmpeg(Job),
    Transcribe(TranscribeJob),
    FfmpegArgs {
        args: Vec<String>,
        duration_s: Option<f64>,
    },
    BurnSubtitles {
        job: Box<Job>,
        srt: PathBuf,
        pipeline_id: String,
    },
    FfmpegTemplate {
        input: PathBuf,
        output: PathBuf,
        template: JobTemplate,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueItem {
    pub id: usize,
    pub description: String,
    pub step: StepKind,
    pub status: StepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_s: Option<f64>,
    pub progress: f64,
    pub logs: Vec<String>,
    pub exit_code: Option<i32>,
    pub error_message: Option<String>,
    #[serde(default)]
    pub depends_on: Option<usize>,
}

impl QueueItem {
    pub fn new(id: usize, description: impl Into<String>, step: StepKind) -> Self {
        Self {
            id,
            description: description.into(),
            step,
            status: StepStatus::Pending,
            duration_s: None,
            progress: 0.0,
            logs: Vec::new(),
            exit_code: None,
            error_message: None,
            depends_on: None,
        }
    }
}

#[derive(Debug)]
pub struct JobQueue {
    pub items: Vec<QueueItem>,
    next_id: usize,
}

impl Default for JobQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl JobQueue {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            next_id: 1,
        }
    }

    pub fn push(&mut self, description: impl Into<String>, step: StepKind) -> usize {
        self.push_with_duration(description, step, None)
    }

    pub fn push_with_duration(
        &mut self,
        description: impl Into<String>,
        step: StepKind,
        duration_s: Option<f64>,
    ) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let mut item = QueueItem::new(id, description, step);
        item.duration_s = duration_s;
        self.items.push(item);
        id
    }

    pub fn push_dependent(
        &mut self,
        description: impl Into<String>,
        step: StepKind,
        duration_s: Option<f64>,
        depends_on: usize,
    ) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        let mut item = QueueItem::new(id, description, step);
        item.duration_s = duration_s;
        item.depends_on = Some(depends_on);
        self.items.push(item);
        id
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn has_pending(&self) -> bool {
        self.items.iter().any(|item| item.status == StepStatus::Pending)
    }

    pub fn has_runnable(&self) -> bool {
        self.next_runnable_id().is_some()
    }

    pub fn is_dependency_satisfied(&self, item: &QueueItem) -> bool {
        if let Some(dep_id) = item.depends_on {
            if let Some(dep) = self.items.iter().find(|i| i.id == dep_id) {
                dep.status == StepStatus::Finished
            } else {
                false // Prerequisite item is missing
            }
        } else {
            true
        }
    }

    pub fn is_dependency_broken(&self, item: &QueueItem) -> bool {
        if let Some(dep_id) = item.depends_on {
            if let Some(dep) = self.items.iter().find(|i| i.id == dep_id) {
                matches!(dep.status, StepStatus::Failed | StepStatus::Cancelled)
            } else {
                true // Prerequisite item was removed
            }
        } else {
            false
        }
    }

    /// Propagates failure or cancellation to dependent items, preventing queue deadlocks
    pub fn cascade_cancel_dependencies(&mut self) -> Vec<usize> {
        let mut cancelled_ids = Vec::new();
        loop {
            let mut changed = false;
            for i in 0..self.items.len() {
                if self.items[i].status == StepStatus::Pending {
                    let dep_id = self.items[i].depends_on;
                    let broken = if let Some(d_id) = dep_id {
                        if let Some(dep) = self.items.iter().find(|it| it.id == d_id) {
                            matches!(dep.status, StepStatus::Failed | StepStatus::Cancelled)
                        } else {
                            true
                        }
                    } else {
                        false
                    };

                    if broken {
                        self.items[i].status = StepStatus::Cancelled;
                        self.items[i].error_message = Some(format!(
                            "Annullato perché il lavoro da cui dipendeva (#{}) è fallito, annullato o mancante",
                            dep_id.unwrap_or(0)
                        ));
                        cancelled_ids.push(self.items[i].id);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        cancelled_ids
    }

    pub fn next_runnable_id(&self) -> Option<usize> {
        self.items
            .iter()
            .find(|item| item.status == StepStatus::Pending && self.is_dependency_satisfied(item))
            .map(|item| item.id)
    }

    pub fn current_pending_mut(&mut self) -> Option<&mut QueueItem> {
        self.items.iter_mut().find(|item| item.status == StepStatus::Pending)
    }

    pub fn running_item_mut(&mut self) -> Option<&mut QueueItem> {
        self.items.iter_mut().find(|item| item.status == StepStatus::Running)
    }

    pub fn find_mut(&mut self, id: usize) -> Option<&mut QueueItem> {
        self.items.iter_mut().find(|item| item.id == id)
    }

    pub fn retry(&mut self, id: usize) {
        if let Some(item) = self.find_mut(id) {
            item.status = StepStatus::Pending;
            item.progress = 0.0;
            item.error_message = None;
            item.exit_code = None;
        }
    }

    pub fn move_up(&mut self, id: usize) {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            if pos > 0 {
                self.items.swap(pos, pos - 1);
            }
        }
    }

    pub fn move_down(&mut self, id: usize) {
        if let Some(pos) = self.items.iter().position(|i| i.id == id) {
            if pos + 1 < self.items.len() {
                self.items.swap(pos, pos + 1);
            }
        }
    }

    pub fn remove(&mut self, id: usize) {
        self.items.retain(|item| item.id != id);
    }

    pub fn clear(&mut self) {
        self.items.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_queue_default_starts_at_one() {
        let mut queue = JobQueue::default();
        let id1 = queue.push("Test 1".to_string(), StepKind::FfmpegArgs { args: vec![], duration_s: None });
        assert_eq!(id1, 1);
        let id2 = queue.push("Test 2".to_string(), StepKind::FfmpegArgs { args: vec![], duration_s: None });
        assert_eq!(id2, 2);
    }

    #[test]
    fn test_queue_cascade_cancel_dependencies() {
        let mut queue = JobQueue::default();
        let id1 = queue.push("Step 1".to_string(), StepKind::FfmpegArgs { args: vec![], duration_s: None });
        let id2 = queue.push_dependent("Step 2".to_string(), StepKind::FfmpegArgs { args: vec![], duration_s: None }, None, id1);
        let id3 = queue.push_dependent("Step 3".to_string(), StepKind::FfmpegArgs { args: vec![], duration_s: None }, None, id2);

        assert_eq!(queue.next_runnable_id(), Some(id1));

        // Mark id1 as failed
        queue.find_mut(id1).unwrap().status = StepStatus::Failed;
        let cancelled = queue.cascade_cancel_dependencies();
        assert_eq!(cancelled, vec![id2, id3]);

        assert_eq!(queue.find_mut(id2).unwrap().status, StepStatus::Cancelled);
        assert_eq!(queue.find_mut(id3).unwrap().status, StepStatus::Cancelled);
        assert!(!queue.has_runnable());
    }

    #[test]
    fn test_queue_missing_dependency_is_not_satisfied() {
        let mut queue = JobQueue::default();
        let id1 = queue.push("Step 1".to_string(), StepKind::FfmpegArgs { args: vec![], duration_s: None });
        let id2 = queue.push_dependent("Step 2".to_string(), StepKind::FfmpegArgs { args: vec![], duration_s: None }, None, id1);

        queue.remove(id1);
        assert_eq!(queue.next_runnable_id(), None);
        let item2 = queue.items.iter().find(|i| i.id == id2).unwrap();
        assert!(queue.is_dependency_broken(item2));
    }
}

