//! Curated catalog of downloadable local GGUF models (≈400 model × quantization variants)
//! with size and hardware requirement estimates. Downloads resolve the exact file name via the
//! Hugging Face API at download time, so a catalog entry only needs repo + quantization.

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cat {
    Chat,
    Code,
    Reasoning,
    Vision,
    Tiny,
}

impl Cat {
    pub const ALL: [Cat; 5] = [Cat::Chat, Cat::Code, Cat::Reasoning, Cat::Vision, Cat::Tiny];
    pub fn label(self) -> &'static str {
        match self {
            Cat::Chat => crate::i18n::t("Chat", "Sohbet"),
            Cat::Code => crate::i18n::t("Code", "Kod"),
            Cat::Reasoning => crate::i18n::t("Reasoning", "Akıl yürütme"),
            Cat::Vision => crate::i18n::t("Vision", "Görsel"),
            Cat::Tiny => crate::i18n::t("Tiny", "Küçük"),
        }
    }
}

/// (display name, Hugging Face repo, total params in B, active params in B (MoE) or 0, category, context in K tokens)
type Row = (&'static str, &'static str, f32, f32, Cat, u32);

const MODELS: &[Row] = &[
    // ---- Meta Llama
    (
        "Llama 3.2 1B Instruct",
        "bartowski/Llama-3.2-1B-Instruct-GGUF",
        1.24,
        0.0,
        Cat::Tiny,
        128,
    ),
    (
        "Llama 3.2 3B Instruct",
        "bartowski/Llama-3.2-3B-Instruct-GGUF",
        3.21,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Llama 3.1 8B Instruct",
        "bartowski/Meta-Llama-3.1-8B-Instruct-GGUF",
        8.03,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Llama 3 8B Instruct",
        "bartowski/Meta-Llama-3-8B-Instruct-GGUF",
        8.03,
        0.0,
        Cat::Chat,
        8,
    ),
    (
        "Llama 3.3 70B Instruct",
        "bartowski/Llama-3.3-70B-Instruct-GGUF",
        70.6,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Llama 3.1 70B Instruct",
        "bartowski/Meta-Llama-3.1-70B-Instruct-GGUF",
        70.6,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Llama 3.1 Nemotron 70B",
        "bartowski/Llama-3.1-Nemotron-70B-Instruct-HF-GGUF",
        70.6,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Hermes 3 Llama 3.1 8B",
        "bartowski/Hermes-3-Llama-3.1-8B-GGUF",
        8.03,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Llama 4 Scout 17B-16E",
        "unsloth/Llama-4-Scout-17B-16E-Instruct-GGUF",
        109.0,
        17.0,
        Cat::Vision,
        1024,
    ),
    (
        "TinyLlama 1.1B Chat",
        "TheBloke/TinyLlama-1.1B-Chat-v1.0-GGUF",
        1.1,
        0.0,
        Cat::Tiny,
        2,
    ),
    // ---- Qwen 2.5
    (
        "Qwen2.5 0.5B Instruct",
        "bartowski/Qwen2.5-0.5B-Instruct-GGUF",
        0.49,
        0.0,
        Cat::Tiny,
        32,
    ),
    (
        "Qwen2.5 1.5B Instruct",
        "bartowski/Qwen2.5-1.5B-Instruct-GGUF",
        1.54,
        0.0,
        Cat::Tiny,
        32,
    ),
    (
        "Qwen2.5 3B Instruct",
        "bartowski/Qwen2.5-3B-Instruct-GGUF",
        3.09,
        0.0,
        Cat::Chat,
        32,
    ),
    (
        "Qwen2.5 7B Instruct",
        "bartowski/Qwen2.5-7B-Instruct-GGUF",
        7.62,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Qwen2.5 14B Instruct",
        "bartowski/Qwen2.5-14B-Instruct-GGUF",
        14.8,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Qwen2.5 14B Instruct 1M",
        "bartowski/Qwen2.5-14B-Instruct-1M-GGUF",
        14.8,
        0.0,
        Cat::Chat,
        1000,
    ),
    (
        "Qwen2.5 32B Instruct",
        "bartowski/Qwen2.5-32B-Instruct-GGUF",
        32.8,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Qwen2.5 72B Instruct",
        "bartowski/Qwen2.5-72B-Instruct-GGUF",
        72.7,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Qwen2.5 Coder 0.5B",
        "bartowski/Qwen2.5-Coder-0.5B-Instruct-GGUF",
        0.49,
        0.0,
        Cat::Code,
        32,
    ),
    (
        "Qwen2.5 Coder 1.5B",
        "bartowski/Qwen2.5-Coder-1.5B-Instruct-GGUF",
        1.54,
        0.0,
        Cat::Code,
        32,
    ),
    (
        "Qwen2.5 Coder 3B",
        "bartowski/Qwen2.5-Coder-3B-Instruct-GGUF",
        3.09,
        0.0,
        Cat::Code,
        32,
    ),
    (
        "Qwen2.5 Coder 7B",
        "bartowski/Qwen2.5-Coder-7B-Instruct-GGUF",
        7.62,
        0.0,
        Cat::Code,
        128,
    ),
    (
        "Qwen2.5 Coder 14B",
        "bartowski/Qwen2.5-Coder-14B-Instruct-GGUF",
        14.8,
        0.0,
        Cat::Code,
        128,
    ),
    (
        "Qwen2.5 Coder 32B",
        "bartowski/Qwen2.5-Coder-32B-Instruct-GGUF",
        32.8,
        0.0,
        Cat::Code,
        128,
    ),
    (
        "Qwen2.5 Math 7B",
        "bartowski/Qwen2.5-Math-7B-Instruct-GGUF",
        7.62,
        0.0,
        Cat::Reasoning,
        4,
    ),
    (
        "Qwen2 VL 7B",
        "bartowski/Qwen2-VL-7B-Instruct-GGUF",
        8.29,
        0.0,
        Cat::Vision,
        32,
    ),
    (
        "Qwen2.5 VL 7B",
        "unsloth/Qwen2.5-VL-7B-Instruct-GGUF",
        8.29,
        0.0,
        Cat::Vision,
        128,
    ),
    (
        "Qwen2.5 VL 3B",
        "unsloth/Qwen2.5-VL-3B-Instruct-GGUF",
        3.75,
        0.0,
        Cat::Vision,
        128,
    ),
    // ---- Qwen 3
    (
        "Qwen3 0.6B",
        "bartowski/Qwen_Qwen3-0.6B-GGUF",
        0.6,
        0.0,
        Cat::Tiny,
        32,
    ),
    (
        "Qwen3 1.7B",
        "bartowski/Qwen_Qwen3-1.7B-GGUF",
        1.7,
        0.0,
        Cat::Tiny,
        32,
    ),
    (
        "Qwen3 4B",
        "bartowski/Qwen_Qwen3-4B-GGUF",
        4.0,
        0.0,
        Cat::Reasoning,
        32,
    ),
    (
        "Qwen3 8B",
        "bartowski/Qwen_Qwen3-8B-GGUF",
        8.2,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "Qwen3 14B",
        "bartowski/Qwen_Qwen3-14B-GGUF",
        14.8,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "Qwen3 32B",
        "bartowski/Qwen_Qwen3-32B-GGUF",
        32.8,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "Qwen3 30B-A3B (MoE)",
        "bartowski/Qwen_Qwen3-30B-A3B-GGUF",
        30.5,
        3.3,
        Cat::Reasoning,
        128,
    ),
    (
        "Qwen3 4B Instruct 2507",
        "unsloth/Qwen3-4B-Instruct-2507-GGUF",
        4.0,
        0.0,
        Cat::Chat,
        256,
    ),
    (
        "Qwen3 4B Thinking 2507",
        "unsloth/Qwen3-4B-Thinking-2507-GGUF",
        4.0,
        0.0,
        Cat::Reasoning,
        256,
    ),
    (
        "Qwen3 30B-A3B Instruct 2507",
        "unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF",
        30.5,
        3.3,
        Cat::Chat,
        256,
    ),
    (
        "Qwen3 Coder 30B-A3B",
        "unsloth/Qwen3-Coder-30B-A3B-Instruct-GGUF",
        30.5,
        3.3,
        Cat::Code,
        256,
    ),
    (
        "QwQ 32B",
        "bartowski/Qwen_QwQ-32B-GGUF",
        32.8,
        0.0,
        Cat::Reasoning,
        128,
    ),
    // ---- Google Gemma
    (
        "Gemma 3 1B",
        "bartowski/google_gemma-3-1b-it-GGUF",
        1.0,
        0.0,
        Cat::Tiny,
        32,
    ),
    (
        "Gemma 3 4B",
        "bartowski/google_gemma-3-4b-it-GGUF",
        3.9,
        0.0,
        Cat::Vision,
        128,
    ),
    (
        "Gemma 3 12B",
        "bartowski/google_gemma-3-12b-it-GGUF",
        11.8,
        0.0,
        Cat::Vision,
        128,
    ),
    (
        "Gemma 3 27B",
        "bartowski/google_gemma-3-27b-it-GGUF",
        27.0,
        0.0,
        Cat::Vision,
        128,
    ),
    (
        "Gemma 3n E2B",
        "unsloth/gemma-3n-E2B-it-GGUF",
        5.4,
        2.0,
        Cat::Tiny,
        32,
    ),
    (
        "Gemma 3n E4B",
        "unsloth/gemma-3n-E4B-it-GGUF",
        7.9,
        4.0,
        Cat::Chat,
        32,
    ),
    (
        "Gemma 2 2B",
        "bartowski/gemma-2-2b-it-GGUF",
        2.6,
        0.0,
        Cat::Tiny,
        8,
    ),
    (
        "Gemma 2 9B",
        "bartowski/gemma-2-9b-it-GGUF",
        9.2,
        0.0,
        Cat::Chat,
        8,
    ),
    (
        "Gemma 2 27B",
        "bartowski/gemma-2-27b-it-GGUF",
        27.2,
        0.0,
        Cat::Chat,
        8,
    ),
    (
        "Gemma 2 9B SimPO",
        "bartowski/gemma-2-9b-it-SimPO-GGUF",
        9.2,
        0.0,
        Cat::Chat,
        8,
    ),
    // ---- Microsoft Phi
    (
        "Phi-3.5 Mini",
        "bartowski/Phi-3.5-mini-instruct-GGUF",
        3.8,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Phi-4 14B",
        "bartowski/phi-4-GGUF",
        14.7,
        0.0,
        Cat::Reasoning,
        16,
    ),
    (
        "Phi-4 Mini",
        "bartowski/microsoft_Phi-4-mini-instruct-GGUF",
        3.8,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Phi-4 Mini Reasoning",
        "unsloth/Phi-4-mini-reasoning-GGUF",
        3.8,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "Phi-4 Reasoning Plus",
        "unsloth/Phi-4-reasoning-plus-GGUF",
        14.7,
        0.0,
        Cat::Reasoning,
        32,
    ),
    // ---- Mistral
    (
        "Mistral 7B Instruct v0.3",
        "bartowski/Mistral-7B-Instruct-v0.3-GGUF",
        7.25,
        0.0,
        Cat::Chat,
        32,
    ),
    (
        "Mistral Nemo 12B",
        "bartowski/Mistral-Nemo-Instruct-2407-GGUF",
        12.2,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Ministral 8B",
        "bartowski/Ministral-8B-Instruct-2410-GGUF",
        8.0,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Mistral Small 24B (2501)",
        "bartowski/Mistral-Small-24B-Instruct-2501-GGUF",
        23.6,
        0.0,
        Cat::Chat,
        32,
    ),
    (
        "Mistral Small 3.1 24B",
        "bartowski/mistralai_Mistral-Small-3.1-24B-Instruct-2503-GGUF",
        24.0,
        0.0,
        Cat::Vision,
        128,
    ),
    (
        "Mistral Small 3.2 24B",
        "unsloth/Mistral-Small-3.2-24B-Instruct-2506-GGUF",
        24.0,
        0.0,
        Cat::Vision,
        128,
    ),
    (
        "Devstral Small 24B",
        "unsloth/Devstral-Small-2505-GGUF",
        23.6,
        0.0,
        Cat::Code,
        128,
    ),
    (
        "Magistral Small 24B",
        "unsloth/Magistral-Small-2506-GGUF",
        23.6,
        0.0,
        Cat::Reasoning,
        40,
    ),
    (
        "Codestral 22B",
        "bartowski/Codestral-22B-v0.1-GGUF",
        22.2,
        0.0,
        Cat::Code,
        32,
    ),
    (
        "Mixtral 8x7B Instruct",
        "TheBloke/Mixtral-8x7B-Instruct-v0.1-GGUF",
        46.7,
        12.9,
        Cat::Chat,
        32,
    ),
    // ---- DeepSeek
    (
        "DeepSeek R1 Distill Qwen 1.5B",
        "bartowski/DeepSeek-R1-Distill-Qwen-1.5B-GGUF",
        1.78,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "DeepSeek R1 Distill Qwen 7B",
        "bartowski/DeepSeek-R1-Distill-Qwen-7B-GGUF",
        7.62,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "DeepSeek R1 Distill Qwen 14B",
        "bartowski/DeepSeek-R1-Distill-Qwen-14B-GGUF",
        14.8,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "DeepSeek R1 Distill Qwen 32B",
        "bartowski/DeepSeek-R1-Distill-Qwen-32B-GGUF",
        32.8,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "DeepSeek R1 Distill Llama 8B",
        "bartowski/DeepSeek-R1-Distill-Llama-8B-GGUF",
        8.03,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "DeepSeek R1 Distill Llama 70B",
        "bartowski/DeepSeek-R1-Distill-Llama-70B-GGUF",
        70.6,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "DeepSeek R1 0528 Qwen3 8B",
        "unsloth/DeepSeek-R1-0528-Qwen3-8B-GGUF",
        8.2,
        0.0,
        Cat::Reasoning,
        128,
    ),
    (
        "DeepSeek Coder V2 Lite 16B",
        "bartowski/DeepSeek-Coder-V2-Lite-Instruct-GGUF",
        15.7,
        2.4,
        Cat::Code,
        128,
    ),
    (
        "DeepSeek Coder 6.7B",
        "TheBloke/deepseek-coder-6.7B-instruct-GGUF",
        6.7,
        0.0,
        Cat::Code,
        16,
    ),
    (
        "DeepSeek Coder 1.3B",
        "TheBloke/deepseek-coder-1.3b-instruct-GGUF",
        1.35,
        0.0,
        Cat::Code,
        16,
    ),
    (
        "DeepSeek Coder 33B",
        "TheBloke/deepseek-coder-33B-instruct-GGUF",
        33.3,
        0.0,
        Cat::Code,
        16,
    ),
    // ---- OpenAI open weights
    (
        "gpt-oss 20B",
        "unsloth/gpt-oss-20b-GGUF",
        21.0,
        3.6,
        Cat::Reasoning,
        128,
    ),
    (
        "gpt-oss 120B",
        "unsloth/gpt-oss-120b-GGUF",
        117.0,
        5.1,
        Cat::Reasoning,
        128,
    ),
    // ---- IBM Granite
    (
        "Granite 3.1 2B",
        "bartowski/granite-3.1-2b-instruct-GGUF",
        2.5,
        0.0,
        Cat::Tiny,
        128,
    ),
    (
        "Granite 3.1 8B",
        "bartowski/granite-3.1-8b-instruct-GGUF",
        8.2,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Granite 3.3 8B",
        "unsloth/granite-3.3-8b-instruct-GGUF",
        8.2,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Granite 3.3 2B",
        "unsloth/granite-3.3-2b-instruct-GGUF",
        2.5,
        0.0,
        Cat::Tiny,
        128,
    ),
    // ---- Small models
    (
        "SmolLM2 135M",
        "bartowski/SmolLM2-135M-Instruct-GGUF",
        0.135,
        0.0,
        Cat::Tiny,
        8,
    ),
    (
        "SmolLM2 360M",
        "bartowski/SmolLM2-360M-Instruct-GGUF",
        0.36,
        0.0,
        Cat::Tiny,
        8,
    ),
    (
        "SmolLM2 1.7B",
        "bartowski/SmolLM2-1.7B-Instruct-GGUF",
        1.7,
        0.0,
        Cat::Tiny,
        8,
    ),
    (
        "SmolLM3 3B",
        "unsloth/SmolLM3-3B-GGUF",
        3.1,
        0.0,
        Cat::Chat,
        64,
    ),
    (
        "Falcon3 1B",
        "bartowski/Falcon3-1B-Instruct-GGUF",
        1.7,
        0.0,
        Cat::Tiny,
        8,
    ),
    (
        "Falcon3 3B",
        "bartowski/Falcon3-3B-Instruct-GGUF",
        3.2,
        0.0,
        Cat::Chat,
        32,
    ),
    (
        "Falcon3 7B",
        "bartowski/Falcon3-7B-Instruct-GGUF",
        7.5,
        0.0,
        Cat::Chat,
        32,
    ),
    (
        "Falcon3 10B",
        "bartowski/Falcon3-10B-Instruct-GGUF",
        10.3,
        0.0,
        Cat::Chat,
        32,
    ),
    // ---- Others
    (
        "Command R7B",
        "bartowski/c4ai-command-r7b-12-2024-GGUF",
        8.0,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "Aya Expanse 8B",
        "bartowski/aya-expanse-8b-GGUF",
        8.0,
        0.0,
        Cat::Chat,
        8,
    ),
    (
        "Aya Expanse 32B",
        "bartowski/aya-expanse-32b-GGUF",
        32.3,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "EXAONE 3.5 2.4B",
        "bartowski/EXAONE-3.5-2.4B-Instruct-GGUF",
        2.4,
        0.0,
        Cat::Tiny,
        32,
    ),
    (
        "EXAONE 3.5 7.8B",
        "bartowski/EXAONE-3.5-7.8B-Instruct-GGUF",
        7.8,
        0.0,
        Cat::Chat,
        32,
    ),
    (
        "EXAONE 3.5 32B",
        "bartowski/EXAONE-3.5-32B-Instruct-GGUF",
        32.0,
        0.0,
        Cat::Chat,
        32,
    ),
    (
        "OLMo 2 7B",
        "bartowski/OLMo-2-1124-7B-Instruct-GGUF",
        7.3,
        0.0,
        Cat::Chat,
        4,
    ),
    (
        "OLMo 2 13B",
        "bartowski/OLMo-2-1124-13B-Instruct-GGUF",
        13.7,
        0.0,
        Cat::Chat,
        4,
    ),
    (
        "InternLM 2.5 7B",
        "bartowski/internlm2_5-7b-chat-GGUF",
        7.7,
        0.0,
        Cat::Chat,
        32,
    ),
    (
        "Yi 1.5 9B",
        "bartowski/Yi-1.5-9B-Chat-GGUF",
        8.8,
        0.0,
        Cat::Chat,
        4,
    ),
    (
        "Yi 1.5 34B",
        "bartowski/Yi-1.5-34B-Chat-GGUF",
        34.4,
        0.0,
        Cat::Chat,
        4,
    ),
    (
        "GLM-4 9B",
        "bartowski/glm-4-9b-chat-GGUF",
        9.4,
        0.0,
        Cat::Chat,
        128,
    ),
    (
        "GLM-4 32B",
        "unsloth/GLM-4-32B-0414-GGUF",
        32.6,
        0.0,
        Cat::Code,
        32,
    ),
    (
        "OpenChat 3.6 8B",
        "bartowski/openchat-3.6-8b-20240522-GGUF",
        8.03,
        0.0,
        Cat::Chat,
        8,
    ),
    (
        "Starling LM 7B",
        "bartowski/Starling-LM-7B-beta-GGUF",
        7.24,
        0.0,
        Cat::Chat,
        8,
    ),
    (
        "Zephyr 7B Beta",
        "TheBloke/zephyr-7B-beta-GGUF",
        7.24,
        0.0,
        Cat::Chat,
        32,
    ),
    (
        "Nous Hermes 2 Mistral 7B",
        "NousResearch/Nous-Hermes-2-Mistral-7B-DPO-GGUF",
        7.24,
        0.0,
        Cat::Chat,
        32,
    ),
    (
        "StarCoder2 15B Instruct",
        "bartowski/starcoder2-15b-instruct-v0.1-GGUF",
        16.0,
        0.0,
        Cat::Code,
        16,
    ),
    (
        "CodeGemma 7B",
        "bartowski/codegemma-1.1-7b-it-GGUF",
        8.5,
        0.0,
        Cat::Code,
        8,
    ),
    (
        "CodeLlama 7B Instruct",
        "TheBloke/CodeLlama-7B-Instruct-GGUF",
        6.74,
        0.0,
        Cat::Code,
        16,
    ),
    (
        "CodeLlama 13B Instruct",
        "TheBloke/CodeLlama-13B-Instruct-GGUF",
        13.0,
        0.0,
        Cat::Code,
        16,
    ),
    (
        "Llava 1.6 Mistral 7B",
        "cjpais/llava-1.6-mistral-7b-gguf",
        7.57,
        0.0,
        Cat::Vision,
        32,
    ),
    (
        "MiniCPM-V 2.6",
        "openbmb/MiniCPM-V-2_6-gguf",
        8.1,
        0.0,
        Cat::Vision,
        32,
    ),
];

/// Quantizations with approximate bits per weight.
pub const QUANTS: &[(&str, f32, &str)] = &[
    ("Q2_K", 3.0, "smallest, lower quality"),
    ("Q3_K_M", 3.9, "small"),
    ("Q4_K_M", 4.85, "recommended"),
    ("Q5_K_M", 5.7, "high quality"),
    ("Q6_K", 6.6, "very high quality"),
    ("Q8_0", 8.5, "near lossless"),
];

#[derive(Clone, Debug)]
pub struct Entry {
    pub name: &'static str,
    pub repo: &'static str,
    pub params_b: f32,
    pub active_b: f32,
    pub cat: Cat,
    pub ctx_k: u32,
    pub quant: &'static str,
    pub bpw: f32,
    pub note: &'static str,
}

impl Entry {
    pub fn size_gb(&self) -> f32 {
        self.params_b * self.bpw / 8.0 * 1.04 + 0.05
    }
    /// RAM needed to run on CPU with ~8K context.
    pub fn ram_gb(&self) -> f32 {
        (self.size_gb() * 1.15 + 0.8 + self.params_b.min(70.0) * 0.02).ceil()
    }
    /// VRAM for full GPU offload.
    pub fn vram_gb(&self) -> f32 {
        (self.size_gb() + 0.6 + self.params_b.min(70.0) * 0.015).ceil()
    }
    pub fn recommended(&self) -> bool {
        self.quant == "Q4_K_M"
    }
}

pub fn entries() -> Vec<Entry> {
    let mut out = Vec::with_capacity(MODELS.len() * 6);
    for &(name, repo, params_b, active_b, cat, ctx_k) in MODELS {
        for &(quant, bpw, note) in QUANTS {
            // Huge models: skip the biggest quants. Tiny models: skip the lossy ones.
            if params_b > 40.0 && bpw > 5.0 {
                continue;
            }
            if params_b < 2.0 && bpw < 4.0 {
                continue;
            }
            out.push(Entry {
                name,
                repo,
                params_b,
                active_b,
                cat,
                ctx_k,
                quant,
                bpw,
                note,
            });
        }
    }
    out
}

/// Fit of a model on this machine.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    Good,
    Tight,
    TooBig,
}

pub fn fit(e: &Entry, ram_gb: f32) -> Fit {
    let need = e.ram_gb();
    if need <= ram_gb * 0.7 {
        Fit::Good
    } else if need <= ram_gb * 0.95 {
        Fit::Tight
    } else {
        Fit::TooBig
    }
}

/// Total system RAM in GB.
pub fn system_ram_gb() -> f32 {
    let mut s = sysinfo::System::new();
    s.refresh_memory();
    s.total_memory() as f32 / 1_073_741_824.0
}

/// Pick the matching `.gguf` files for a quantization from a repo listing
/// (single file preferred; all parts of a split model otherwise).
pub fn pick_files(files: &[(String, u64)], quant: &str) -> Vec<(String, u64)> {
    let q = quant.to_ascii_lowercase();
    let mut matches: Vec<(String, u64)> = files
        .iter()
        .filter(|(p, _)| {
            let l = p.to_ascii_lowercase();
            l.ends_with(".gguf")
                && !l.contains("mmproj")
                && (l.contains(&format!("-{q}"))
                    || l.contains(&format!(".{q}"))
                    || l.contains(&format!("_{q}")))
                && !l.contains(&format!("{q}_l"))
                && !l.contains(&format!("{q}_xl"))
        })
        .cloned()
        .collect();
    matches.sort();
    if let Some(single) = matches.iter().find(|(p, _)| !p.contains("-of-")) {
        return vec![single.clone()];
    }
    matches
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_big_and_sane() {
        let e = entries();
        assert!(e.len() >= 300 && e.len() <= 700, "{}", e.len());
        let q = e
            .iter()
            .find(|x| x.repo.contains("Qwen2.5-Coder-7B") && x.quant == "Q4_K_M")
            .unwrap();
        assert!((q.size_gb() - 4.8).abs() < 0.5, "{}", q.size_gb());
        assert!(q.ram_gb() >= 6.0);
    }

    #[test]
    fn picks_files() {
        let files = vec![
            ("Qwen2.5-7B-Instruct-Q4_K_L.gguf".to_string(), 1),
            ("Qwen2.5-7B-Instruct-Q4_K_M.gguf".to_string(), 2),
            ("Qwen2.5-7B-Instruct-Q8_0.gguf".to_string(), 3),
        ];
        assert_eq!(
            pick_files(&files, "Q4_K_M")[0].0,
            "Qwen2.5-7B-Instruct-Q4_K_M.gguf"
        );
        let split = vec![
            ("q4_k_m/x-q4_k_m-00002-of-00002.gguf".to_string(), 1),
            ("q4_k_m/x-q4_k_m-00001-of-00002.gguf".to_string(), 1),
        ];
        let p = pick_files(&split, "Q4_K_M");
        assert_eq!(p.len(), 2);
        assert!(p[0].0.contains("00001"));
        let thebloke = vec![("tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf".to_string(), 1)];
        assert_eq!(pick_files(&thebloke, "Q4_K_M").len(), 1);
    }
}
