//! Logos for AI providers (Lobe Icons, MIT; see `assets/brand/providers/`).
//! All are one-colour shapes that follow the text colour, so they work in
//! light and dark themes. A provider without a logo gets a letter tile from
//! `cli_logo::tile`. Several provider ids can share one file (regional or
//! plan variants of the same company).

use std::borrow::Cow;

macro_rules! logos {
    ($($id:literal => $file:literal),* $(,)?) => {
        /// (provider id, asset path, bytes).
        const LOGOS: &[(&str, &str, &[u8])] = &[$((
            $id,
            concat!("brand/providers/", $file, ".svg"),
            include_bytes!(concat!("../../../assets/brand/providers/", $file, ".svg")),
        )),*];
    };
}

logos! {
    "openai" => "openai",
    "anthropic" => "anthropic",
    "google" => "gemini",
    "google-vertex" => "vertexai",
    "google-vertex-anthropic" => "vertexai",
    "xai" => "xai",
    "deepseek" => "deepseek",
    "mistral" => "mistral",
    "groq" => "groq",
    "openrouter" => "openrouter",
    "perplexity" => "perplexity",
    "perplexity-agent" => "perplexity",
    "cohere" => "cohere",
    "togetherai" => "together",
    "fireworks-ai" => "fireworks",
    "cerebras" => "cerebras",
    "nvidia" => "nvidia",
    "huggingface" => "huggingface",
    "azure" => "azure",
    "azure-cognitive-services" => "azure",
    "amazon-bedrock" => "bedrock",
    "github-copilot" => "githubcopilot",
    "ollama-cloud" => "ollama",
    "lmstudio" => "lmstudio",
    "moonshotai" => "moonshot",
    "moonshotai-cn" => "moonshot",
    "zhipuai" => "zhipu",
    "zhipuai-coding-plan" => "zhipu",
    "minimax" => "minimax",
    "minimax-cn" => "minimax",
    "minimax-coding-plan" => "minimax",
    "siliconflow" => "siliconcloud",
    "siliconflow-cn" => "siliconcloud",
    "deepinfra" => "deepinfra",
    "ai21" => "ai21",
    "nebius" => "nebius",
    "novita-ai" => "novita",
    "cloudflare-workers-ai" => "cloudflare",
    "cloudflare-ai-gateway" => "cloudflare",
    "vercel" => "vercel",
    "alibaba" => "alibabacloud",
    "alibaba-cn" => "alibabacloud",
    "alibaba-coding-plan" => "alibabacloud",
    "stepfun" => "stepfun",
    "volcengine" => "volcengine",
    "baseten" => "baseten",
    "modelscope" => "modelscope",
    "upstage" => "upstage",
    "meta" => "meta",
    "llama" => "meta",
    "snowflake-cortex" => "snowflake",
    "poe" => "poe",
    "tencent-coding-plan" => "tencent",
}

/// Bytes for a bundled logo path, for `AppAssets`.
pub fn load(path: &str) -> Option<Cow<'static, [u8]>> {
    LOGOS
        .iter()
        .find(|l| l.1 == path)
        .map(|l| Cow::Borrowed(l.2))
}

/// Asset paths under `prefix`, for `AppAssets::list`.
pub fn list(prefix: &str) -> impl Iterator<Item = &'static str> + '_ {
    LOGOS
        .iter()
        .map(|l| l.1)
        .filter(move |p| p.starts_with(prefix))
}

/// The asset path of a provider's logo, if it has one.
pub fn path_for(provider_id: &str) -> Option<&'static str> {
    LOGOS.iter().find(|l| l.0 == provider_id).map(|l| l.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_listed_provider_resolves_to_bundled_bytes() {
        for (id, path, bytes) in LOGOS {
            assert_eq!(path_for(id), Some(*path));
            assert!(load(path).is_some() && bytes.starts_with(b"<svg"), "{id}");
        }
    }
}
