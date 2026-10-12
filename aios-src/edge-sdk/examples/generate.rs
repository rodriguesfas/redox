//! Demo: load a model via the SDK and run real inference.
//!
//! Usage:
//!   cargo run -p aios-sdk --example generate [model] [prompt] [max_tokens]
//!
//! `model` is a name resolved by `aios_sdk::load` (registry or cache dir
//! `AIOS_MODELS_DIR`). The model must have a `tokenizer.json` next to it.

use aios_sdk::{generate_with_metrics, load};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let model_name = args
        .get(1)
        .map(|s| s.as_str())
        .unwrap_or("tinyllama.q4_k_m");
    let prompt = args
        .get(2)
        .map(|s| s.as_str())
        .unwrap_or("The capital of France is");
    let max_tokens = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(16);

    let model = load(model_name).expect("aios_sdk::load");
    let gen = generate_with_metrics(&model, prompt, max_tokens).expect("generate");

    println!("model:  {}", model.name);
    println!("loaded: {}", model.path);
    println!("prompt: {prompt}");
    println!("text:   {}", gen.text);
    println!(
        "load:   {:.0} ms ({:.2} t/s)",
        gen.load_ms, gen.tokens_per_second
    );
}
