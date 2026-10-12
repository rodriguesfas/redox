//! AITask framework: built-in task specs + config-driven routing (task -> model).
//!
//! A task is a named, reusable prompt template plus a model binding. Built-ins
//! ship with the CLI; a JSON config file (`AIOS_TASKS` or `/etc/ai/tasks.json`)
//! can override their model/prompt/max_tokens or add brand-new tasks.

use std::collections::HashMap;

use serde::Deserialize;

/// Build the JSON CLI payload for `--json` mode.
pub fn task_json(name: &str, model: &str, text: &str, output: &str, tps: f64) -> String {
    serde_json::json!({
        "task": name,
        "model": model,
        "text": text,
        "output": output,
        "tokens_per_second": tps,
    })
    .to_string()
}

/// A task instance ready to run: template + model binding + limits.
#[derive(Debug, Clone)]
pub struct TaskSpec {
    pub name: String,
    pub description: String,
    /// Prompt template; `{input}` is replaced with the task input text.
    pub prompt: String,
    pub max_tokens: usize,
    /// Model binding from routing config or CLI override (None = caller decides).
    pub model: Option<String>,
}

impl TaskSpec {
    /// Fill the input placeholder and produce the final prompt.
    pub fn render(&self, input: &str) -> String {
        self.prompt.replace("{input}", input)
    }
}

fn builtin(name: &str, description: &str, prompt: &str, max_tokens: usize) -> TaskSpec {
    TaskSpec {
        name: name.to_string(),
        description: description.to_string(),
        prompt: prompt.to_string(),
        max_tokens,
        model: None,
    }
}

/// The built-in offline-capable tasks (CPU-first).
fn builtins() -> Vec<TaskSpec> {
    vec![
        builtin(
            "summarize",
            "summarize a text into short bullet points",
            "Summarize the following text in a few short bullet points.\nText:\n{input}",
            128,
        ),
        builtin(
            "classify",
            "classify sentiment of a text (positive/negative/neutral)",
            "Classify the sentiment of the text below. Answer with exactly one word: positive, negative, or neutral.\nText:\n{input}",
            16,
        ),
        builtin(
            "intent",
            "classify user intent of a message in one short phrase",
            "What does the user intend with this message? Answer with one short phrase.\nMessage:\n{input}",
            24,
        ),
        builtin(
            "translate",
            "translate a text into English",
            "Translate the following text into English. Keep it faithful and do not add comments.\nText:\n{input}",
            192,
        ),
        builtin(
            "qa",
            "answer a question based on the provided context",
            "Answer the question based only on the context provided. If the answer is not in the context, say so.\nContext:\n{input}",
            160,
        ),
        builtin(
            "correct",
            "fix grammar, spelling, and style of a text",
            "Correct grammar, spelling, and punctuation in the following text. Preserve the original meaning and tone. Output only the corrected text.\nText:\n{input}",
            128,
        ),
        builtin(
            "rewrite",
            "paraphrase or rewrite a text with different style/tone",
            "Rewrite the following text to improve clarity and flow while preserving the original meaning. Output only the rewritten text.\nText:\n{input}",
            192,
        ),
        builtin(
            "extract",
            "extract named entities, keywords, or key phrases from text",
            "Extract the key entities, concepts, and phrases from the following text. Output as a JSON array of strings.\nText:\n{input}",
            128,
        ),
        builtin(
            "code",
            "explain, review, or generate code from a description",
            "You are a helpful programming assistant. {input}\n\nProvide clear, concise code with brief explanations where helpful.",
            256,
        ),
        builtin(
            "autocomplete",
            "complete a code snippet or continue a partial implementation",
            "Complete the following code snippet. Output only the completion, no explanations.\nCode:\n{input}",
            128,
        ),
        builtin(
            "review",
            "review code for bugs, style, and improvements",
            "Review the following code for bugs, performance issues, security concerns, and style. Output as bullet points.\nCode:\n{input}",
            256,
        ),
        builtin(
            "explain",
            "explain a concept, code, or text in simple terms",
            "Explain the following in clear, simple terms as if teaching a beginner. Use analogies where helpful.\nTopic:\n{input}",
            192,
        ),
    ]
}

/// A single config route that can override/extend a built-in task.
#[derive(Debug, Clone, Deserialize, Default)]
struct Route {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    max_tokens: Option<usize>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct TaskConfig {
    #[serde(default)]
    routes: HashMap<String, Route>,
}

/// Task registry: built-ins, optionally overridden/added by a config file.
#[derive(Debug, Clone)]
pub struct TaskRouter {
    tasks: HashMap<String, TaskSpec>,
}

impl TaskRouter {
    /// Load routing config from `path` (missing/empty file = built-ins only).
    pub fn load(path: &str) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(contents) => Self::from_str(&contents),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::builtins_only()),
            Err(e) => Err(format!("read {path}: {e}")),
        }
    }

    /// Parse a config document and merge it over the built-ins.
    pub fn from_str(contents: &str) -> Result<Self, String> {
        let config: TaskConfig = if contents.trim().is_empty() {
            TaskConfig::default()
        } else {
            serde_json::from_str(contents).map_err(|e| format!("tasks config: {e}"))?
        };
        let mut tasks = HashMap::new();
        for b in builtins() {
            tasks.insert(b.name.clone(), b);
        }
        for (name, route) in config.routes {
            let spec = match tasks.get_mut(&name) {
                Some(spec) => {
                    if let Some(model) = route.model {
                        spec.model = Some(model);
                    }
                    if let Some(desc) = route.description {
                        spec.description = desc;
                    }
                    if let Some(prompt) = route.prompt {
                        spec.prompt = prompt;
                    }
                    if let Some(max_tokens) = route.max_tokens {
                        spec.max_tokens = max_tokens;
                    }
                    // keep the (possibly updated) spec
                    spec.clone()
                }
                None => {
                    let prompt = route
                        .prompt
                        .ok_or_else(|| format!("task '{name}': new tasks need a \"prompt\""))?;
                    TaskSpec {
                        name: name.clone(),
                        description: route.description.unwrap_or_default(),
                        prompt,
                        max_tokens: route.max_tokens.unwrap_or(128),
                        model: route.model,
                    }
                }
            };
            tasks.insert(name, spec);
        }
        Ok(TaskRouter { tasks })
    }

    fn builtins_only() -> Self {
        let mut tasks = HashMap::new();
        for b in builtins() {
            tasks.insert(b.name.clone(), b);
        }
        TaskRouter { tasks }
    }

    pub fn get(&self, name: &str) -> Option<&TaskSpec> {
        self.tasks.get(name)
    }

    /// All tasks, sorted by name, for `ai task --list`.
    pub fn all(&self) -> Vec<&TaskSpec> {
        let mut v: Vec<&TaskSpec> = self.tasks.values().collect();
        v.sort_by(|a, b| a.name.cmp(&b.name));
        v
    }

    /// Default routing config path (env `AIOS_TASKS` or `/etc/ai/tasks.json`).
    pub fn default_tasks_file() -> String {
        std::env::var("AIOS_TASKS").unwrap_or_else(|_| "/etc/ai/tasks.json".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_render_placeholder() {
        let router = TaskRouter::from_str("").unwrap();
        let s = router.get("summarize").expect("builtin summarize");
        let prompt = s.render("hello world");
        assert!(prompt.contains("hello world"));
        assert!(prompt.contains("{input}") == false);
    }

    #[test]
    fn route_overrides_and_adds() {
        let cfg = r#"{
            "routes": {
                "summarize": { "model": "tinyllama", "max_tokens": 32 },
                "mytask":   { "model": "other", "prompt": "Do: {input}", "description": "custom", "max_tokens": 9 }
            }
        }"#;
        let router = TaskRouter::from_str(cfg).unwrap();
        let s = router.get("summarize").unwrap();
        assert_eq!(s.model.as_deref(), Some("tinyllama"));
        assert_eq!(s.max_tokens, 32);
        let t = router.get("mytask").expect("new task from config");
        assert_eq!(t.description, "custom");
        assert_eq!(t.prompt, "Do: {input}");
        assert_eq!(t.max_tokens, 9);
        assert_eq!(t.render("x"), "Do: x");
    }

    #[test]
    fn empty_and_missing_config_callers() {
        assert!(TaskRouter::from_str("").unwrap().get("classify").is_some());
        assert!(TaskRouter::from_str("  ").unwrap().get("intent").is_some());
        // new task without prompt is rejected
        let bad = r#"{"routes":{"new":{"model":"m"}}}"#;
        assert!(TaskRouter::from_str(bad).is_err());
    }
}
